# rhp bytecode — authoritative build spec

Status: **build spec (implement from this).** Target semantics: PHP 8.5 (ground
truth: the reference `php`, see `crates/rhp-builtins/src/version.rs`; probed on 8.5.8). This is the single source of truth for the
register-based bytecode compiler + VM that becomes rhp's primary execution
core. Where the four research docs disagree, **this doc wins**; their resolved
decisions are folded in and the losing alternatives are noted inline.

> **Historical note (2026-11).** This spec planned two tiers: the VM and the
> old tree-walker, kept as a differential *oracle* (`rhp-oracle`). The oracle
> has since been removed and the VM is rhp's only engine. The oracle mentions
> left below (the §1 crate table and graph, §2.3, §2.5, §4, the §7 migration
> steps) record the design as planned and executed; §6 and §8 item 10 describe
> the test setup as it is today.
>
> The instruction set and the proto layout have grown since:
> `crates/rhp-bytecode/src/lib.rs` is the authority. The reserved inline-cache
> plumbing (`InlineCache`, `caches`, the `cache` operands), the sketched
> `ProtoFlags`, `DebugInfo.cv_names`, `FnIdx` and the `Refuse` opcode were
> never used and are gone; `Default` is now `ParamDefault { Const(CIdx),
> Init }`. The flags that exist now are in §2.6. `eval` and
> `include`/`require` run (§4 lists them as refused).

Synthesizes: [`isa.md`](./history/isa.md) (instruction set), [`vm.md`](./history/vm.md) (VM
runtime), [`bytecode-prior-art.md`](./history/bytecode-prior-art.md) (Zend/Lua lessons),
[`reorg.md`](./history/reorg.md) (crate layout + migration). Read first and reused
verbatim: [`value-model.md`](./value-model.md), [`php-semantics.md`](./php-semantics.md),
[`ast-surface.md`](./history/ast-surface.md), and the already-built crates below.

Non-negotiable invariants (unchanged): single-threaded (`Rc`/`RefCell`, never
`Arc`); `Value`s are owned `'static`; the arena holds only the AST, consumed and
dropped at compile time (a `FunctionProto` carries **no** `'arena` borrow);
`eval` / `create_function` / `include`/`require` /
`(unset)`-cast are **refused**, structurally in the compiler for the
construct nodes and via `is_refused()` for function names (§4). `eval` is
**never** implemented — no opcode interprets a string as code, by construction.

---

## 1. Crate layout and the ops/conv/Thrown decision

> **Today.** The operators are `rhp_values::ops` (rhp-semantics is gone),
> parsing is `rhp_compiler::parse` (rhp-frontend is gone) and the oracle is
> gone: the engine crates are values, bytecode, compiler, builtins, vm,
> sapi and cli, plus rhp-csrc and rhp-mbfl-sys for vendored C. The README
> has the table and the graph. rhp-bytecode also holds php's stub
> signatures (`rhp_bytecode::signatures`), which the compiler, the VM and
> the builtins read. The compiler does not depend on rhp-builtins:
> `rhp_sapi::compile_file` passes it a `rhp_compiler::Builtins` (the
> prelude source, which native functions exist, the values of builtin
> constants for compile-time evaluation, the request's working directory
> for `__DIR__` of code without a file), so the two build in parallel.
> rhp-builtins owns all of the PHP prelude (`core_prelude.php` and each
> module's `prelude.php`). §1.1 to §1.3 are the plan as it was made.

### 1.1 Decision (one coherent answer, justified)

- **`conv` and `fmt` stay in `rhp-values`** (they already live there; pure,
  no `Thrown`, no move).
- **`Thrown` moves DOWN into `rhp-values`** (`rhp_values::thrown`,
  re-exported as `rhp_values::Thrown`; `rhp-builtins` keeps
  `pub use rhp_values::Thrown;` so every existing `builtins::Thrown` path
  still compiles).
- **`ops` moves into a new `rhp-semantics` crate** (not into `rhp-values`).

**Justification (2 lines):** `ops` returns `Result<Value, Thrown>`, so `Thrown`
must sit at or below `ops`; putting `Thrown` in the leaf `rhp-values` lets
*both* `rhp-semantics` (ops) and `rhp-builtins` depend on it without a
cycle, which in turn lets builtins call `ops::loose_compare` for `max/min/sort`.
Keeping `ops` in its own `rhp-semantics` (rather than in `rhp-values`)
keeps the value model free of operator policy and lets a future `rhp-jit`
depend on `bytecode + semantics + values` alone.

> Rejected alternative (vm.md §0): "keep the VM inside `rhp-runtime` next to
> the oracle, no new crate." Overruled because the task's own migration
> checklist mandates separate `rhp-bytecode` / `rhp-compiler` /
> `rhp-vm` / `rhp-oracle` crates (steps ii, iii, iv, vii), and a
> JIT-lowerable, behavior-free ISA crate must not transitively pull in the
> compiler or the VM. The `reorg.md` 9-crate split is adopted.

### 1.2 Target workspace (9 crates)

| crate | role | depends on |
|---|---|---|
| `rhp-values` | `Value`, `PhpStr`, `PhpArray`, `ArrayKey`, `conv`, `fmt`, **`Thrown`** | `bstr`, `indexmap`, `rustc-hash`, `ordered-float` |
| `rhp-frontend` | `ParsedFile` + arena (mago wrapper) | `mago-*` |
| `rhp-semantics` | operator semantics (`ops`) + shared value helpers (`deref`, `inc_dec_value`, `cast_to_array`) | `rhp-values` |
| `rhp-builtins` | `Host`, `NativeFn`, `Builtin`, `registry`, `constants`, `is_refused`, `dump` | `rhp-values`, `rhp-semantics` |
| `rhp-bytecode` | the ISA: `Instr`, `FunctionProto`, `Program`, exception table — **data only, no behavior** | `rhp-values` |
| `rhp-compiler` | AST → bytecode; **owns the structural refusals** (§4) | `rhp-frontend`, `rhp-bytecode`, `rhp-values` |
| `rhp-vm` | the register VM; executes a `Program`, `impl Host for Vm` | `rhp-bytecode`, `rhp-semantics`, `rhp-builtins`, `rhp-values` |
| `rhp-oracle` | the tree-walker (ex-`rhp-runtime`), `walk_*` methods | `rhp-frontend`, `rhp-semantics`, `rhp-builtins`, `rhp-values` |
| `rhp-cli` | SAPI + differential runner (php ⟷ vm ⟷ oracle) | all of the above |

### 1.3 Dependency graph (acyclic)

```
                         rhp-cli
        ┌──────────┬──────────┼──────────┬───────────┐
        v          v          v          v           v
  rhp-compiler rhp-vm  rhp-oracle   rhp-frontend
     │   │          │  │ │        │  │
     │   │          │  │ └────> rhp-builtins ──┐
     │   │          │  └──────> rhp-semantics <┤
     │   └──────────┴────────> rhp-bytecode    │
     └────────────────┬─────────────────┬─────────┘
                      v                  v
                 rhp-values  (leaf: Thrown, conv, fmt, Value)
```

Edge rules that must hold: `rhp-bytecode` depends on **only** `rhp-values`
(so `rhp-jit` can later lower the same ISA without the compiler/VM);
`rhp-compiler` does **not** depend on `rhp-vm`/`-oracle`/`-builtins` (it
refuses only *syntactic constructs*; function-name refusal is a runtime concern,
but see §4.2 — the compiler may optionally link `rhp-builtins` read-only to
fold `is_refused()` at compile time for statically-named calls; if that edge is
taken it is `compiler → builtins`, still acyclic); `rhp-vm` does not depend
on `rhp-frontend` (runs bytecode, never the arena); the two tiers never see
each other — only `rhp-cli` knits them.

### 1.4 Reused verbatim (confirmed signatures — do not reimplement)

```rust
// rhp_values::ops  (was rhp-semantics::ops; first moved from rhp-runtime/src/ops.rs)
pub fn add/sub/mul/div/modulo/pow(a: &Value, b: &Value) -> Result<Value, Thrown>;
pub fn negate(a: &Value) -> Result<Value, Thrown>;
pub fn bitwise(a: &Value, b: &Value, op: BitOp) -> Result<Value, Thrown>; // BitOp::{And,Or,Xor,Shl,Shr,Mod}
pub fn bitwise_not(a: &Value) -> Result<Value, Thrown>;
pub fn concat(a: &Value, b: &Value) -> Value;
pub fn loose_compare(a: &Value, b: &Value) -> std::cmp::Ordering;
pub fn loose_eq(a: &Value, b: &Value) -> bool;
pub fn strict_eq(a: &Value, b: &Value) -> bool;
pub fn spaceship(a: &Value, b: &Value) -> Value;            // Int -1/0/1
// + lifted helpers: deref, inc_dec_value, cast_to_array

// rhp-values::conv / ::fmt
conv::{to_bool,to_string_bytes,to_int,to_float,to_number,numeric_string,is_numeric,scan_number}
fmt::{float_echo (precision-14), float_serialize (serialize_precision -1)}

// rhp-builtins
pub enum Thrown { Error{class:&'static str, message:String}, Unsupported{feature:String, reason:&'static str}, Exit(i32) }  // (def'n now in rhp-values)
pub trait Host { fn write_out(&mut self, bytes: &[u8]); }
pub type NativeFn = fn(&mut dyn Host, &[Value]) -> Result<Value, Thrown>;
pub struct Builtin { /* name, func: NativeFn, min_args, max_args (from the stub signatures) */ }
pub fn registry() -> &'static HashMap<&'static str, Builtin>;
pub fn lookup(name: &str) -> Option<&'static Builtin>;
pub fn constants() -> Vec<(&'static str, Value)>;
pub fn is_refused(name: &str) -> Option<&'static str>;  // Some(reason) ⇒ refuse
```

---

## 2. The locked ISA (`rhp-bytecode`)

### 2.1 Core types

A compilation unit is one **`FunctionProto`** per PHP function/method/closure,
plus one named `"{main}"` for the script body (its CVs are the globals; running
the script is "call `{main}` with no args"). Protos reference each other and the
builtin table only by small integer id — self-contained, `'static`,
`Rc`-shareable.

```rust
use std::rc::Rc;
use rhp_values::{PhpStr, Value, Thrown};

pub type Reg   = u16;  // register-file index: CVs in [0,num_cvs), temporaries above
pub type CIdx  = u32;  // index into FunctionProto.consts
pub type NIdx  = u32;  // index into FunctionProto.names (interned identifier bytes)
pub type PC    = u32;  // instruction index into FunctionProto.code (NOT a byte offset)
pub type FnIdx = u32;  // index into Program.functions
pub type BId   = u32;  // index into the resolved builtin table
pub type CacheIdx = u32; // index into a per-proto inline-cache side table (reserved; §2.5)

pub struct FunctionProto {
    pub name: PhpStr,                   // "{main}" / "{closure}" / declared name
    pub num_params: u16,
    pub params: Vec<ParamInfo>,         // len == num_params
    pub num_cvs: u16,                   // named-local registers: [0,num_cvs), params first
    pub num_registers: u16,             // total frame register-file size (CVs + temps high-water)
    pub code: Vec<Instr>,               // the instruction stream; PC indexes this
    pub consts: Vec<Value>,             // immutable literal / default pool, by CIdx, deduped
    pub names: Vec<PhpStr>,             // interned identifier bytes, by NIdx
    pub exception_table: Vec<ExRegion>, // innermost-first; §3.4
    pub caches: Vec<InlineCache>,       // side table, Cell-based; empty until phase 5/7
    pub debug: DebugInfo,               // pc→line map + cv names; never consulted for semantics
    pub flags: ProtoFlags,              // has_finally, uses_ref, variadic, generator(reserved)
}

pub struct ParamInfo {
    pub name: NIdx,
    pub cv: Reg,                        // CV the param binds (normally == positional index)
    pub by_ref: bool,
    pub variadic: bool,
    pub default: Option<Default>,       // Some ⇒ optional
}
pub enum Default { Const(CIdx), Init(PC) }   // constant default, or run an init block then fall through

pub struct Program {
    pub main: Rc<FunctionProto>,
    pub functions: Vec<Rc<FunctionProto>>,   // FnIdx-indexed; name→FnIdx resolved at compile/link
    // classes: Vec<ClassEntry>  — phase 7
}

pub struct DebugInfo { pub file: PhpStr, pub lines: Vec<(PC,u32)>, pub cv_names: Vec<NIdx> }
pub struct ProtoFlags { pub has_finally: bool, pub uses_ref: bool, pub generator: bool }
```

`Instr` is a **fixed-width `#[derive(Clone, Copy, Debug)] enum`, one variant per
opcode** (decided, isa.md §2.2). Invariant: *every operand is a small `Copy`
index or immediate; no variant owns heap data* — so `Instr: Copy`, ~16 bytes,
`code: Vec<Instr>` is a flat cache-friendly array, dispatch is one exhaustive
`match`, and Cranelift lowering is the *same* `match` emitting IR. Byte-packing
is a future option *behind the same opcode set*; not done now. (Rejected: Lua RK
high-bit packing, Zend's `OP_DATA` trailing-opline, Zend's BP-typed fetch
explosion, CPython inline-interleaved mutable caches.)

### 2.2 Register model (exactly three addressing modes)

One frame = one flat `Vec<Value>` of length `num_registers`, all `Value::Null`
on entry. CVs occupy `[0, num_cvs)` — every named `$var`, params first, gets one
permanent register chosen at compile time; **reading/writing a direct variable
needs no load/store opcode, the register *is* the variable.** Temporaries occupy
`[num_cvs, num_registers)`, allocated by a high-water bump allocator and freed at
expression-statement boundaries; `num_registers` is the peak. A CV slot may hold
`Value::Ref(Rc<RefCell<Value>>)` after `&` promotion; temporaries never hold a
live `Ref` outliving the producing instruction.

| Mode | Encoding | Meaning |
|---|---|---|
| Register | `Reg` (u16) | a frame slot (CV or temp); the only mutable operand space |
| Constant | `CIdx` (u32) | an immutable `Value` in `consts` |
| Immediate | inline scalar in the variant | small ints, bools, `PC`, argc, `NIdx`/`BId`/`FnIdx` |

No instruction addresses memory / an array element / a property directly — those
go through explicit fetch/store opcodes (§2.4 arrays). `deref` (one-level Ref
read) is applied on-use wherever a *value* is required:
`fn deref(v:&Value)->Value { match v { Value::Ref(r)=>r.borrow().clone(), o=>o.clone() } }`.

### 2.3 Delegation rule (load-bearing)

> **Today.** Only two numbers (ints for `% << >> | & ^`) take the `ops::`
> function directly. Everything else goes through the VM's slow path
> (`vm/compare.rs`: `bin_slow`, `long_op`, `bitwise_not_slow`), a port of
> `zend_operators.c` that raises php's diagnostics (they can run a user
> error handler). The engine-free `ops::` functions return an error for
> any operand php would diagnose; constant folding then leaves the
> expression to run time.

Arithmetic / compare / concat / bitwise / unary opcodes are **thin wrappers that
call the matching `ops::` function**; echo/print and all builtins go through
`Host`/`registry()`. The VM contributes only operand fetch, `?`-propagation of
`Thrown`, and the `dst` write. This makes VM and oracle semantically identical by
construction, so any differential mismatch localizes to a *compiler lowering*
bug, never duplicated semantics.

```rust
Instr::Add { dst, a, b } => regs[dst] = ops::add(&get(a), &get(b))?,
Instr::Concat { dst, a, b } => regs[dst] = ops::concat(&get(a), &get(b)),
Instr::Cmp { dst, a, b, kind } => regs[dst] = eval_cmp(kind, &get(a), &get(b)), // wraps ops::loose_*/strict_eq/spaceship
Instr::Echo { src } => { let b = conv::to_string_bytes(&get(src)); self.write_out(&b); }
```

### 2.4 The opcode set (complete, grouped)

Rust enum variants (CamelCase). `?` marks opcodes that may return `Err(Thrown)`
and enter the unwinder (§3.4). Phases per ast-surface.md §14. SCREAMING names
from isa.md are the disassembly spelling of the same variants.

**Move / const / load (phase 1–2)**
```
Nop
Ticks      { n }                  ZEND_TICKS: after each statement under declare(ticks=n); every nth runs the tick functions ?
Mov        { dst, src }           dst = deref(src)                         (value copy; `$b=$a`)
MovTake    { dst, src }           dst = deref(src); src = Null             (a dead temporary into a call window)
LoadConst  { dst, k }             dst = consts[k].clone()
LoadNull   { dst }                dst = Null
LoadBool   { dst, b: bool }       dst = Bool(b)
LoadInt    { dst, n: i64 }        dst = Int(n)                             (inline small int; big ints via LoadConst)
```

**Variable store — ref-aware (phase 2, 6)**
```
Assign     { cv, src }            if regs[cv] is Ref(rc) { *rc.borrow_mut()=deref(src) } else { regs[cv]=deref(src) }
```
Reading a CV needs no opcode. Compound assign `$x += e` lowers to
`Add t,cv,e; Assign cv,t` (reuses `ops::add` + the ref-aware store).

**Arithmetic / bitwise / unary (phase 2) — route to `ops::`**
```
Add Sub Mul { dst,a,b }  → ops::add/sub/mul ?            Div Mod Pow { dst,a,b } → ops::div/modulo/pow ?
BitAnd BitOr BitXor Shl Shr { dst,a,b } → ops::bitwise(..) (no Err)
Concat     { dst,a,b }  → ops::concat (no Err; always Str)
Neg        { dst,a }    → ops::negate ?                  BitNot { dst,a } → ops::bitwise_not
Not        { dst,a }    → Bool(!conv::to_bool(deref(a)))
IncDec     { dst, cv, pre: bool, delta: i8 }  → inc_dec_value, ref-aware write-through; dst = new (pre) or old (post)
```

**Casts (phase 2) — route to `conv::`/helper**
```
CastInt    { dst,a } → Int(conv::to_int)       CastFloat  { dst,a } → Float(conv::to_float)
CastString { dst,a } → Str(conv::to_string_bytes)   CastBool { dst,a } → Bool(conv::to_bool)
CastArray  { dst,a } → cast_to_array(a)        // (object) is phase 7
```
`(unset)`/`(real)` have **no opcode** — refused in the compiler (§4). `(void) expr`
(a statement only, as php's grammar has it) compiles its operand and frees it,
without the unused-result mark below.

**Compare (phase 2) — route to `ops::`** (one op, `kind` selects)
```
Cmp { dst,a,b, kind: CmpKind }   kind ∈ {Eq,Ne,Identical,NotIdentical,Lt,Le,Gt,Ge,Spaceship}
  Eq→Bool(loose_eq)  Ne→Bool(!loose_eq)  Identical→Bool(strict_eq)  NotIdentical→Bool(!strict_eq)
  Lt/Le/Gt/Ge→Bool(loose_compare(a,b).is_lt()/is_le()/is_gt()/is_ge())   Spaceship→ops::spaceship
```
`&&`/`||`/`and`/`or`/`??` are **not** compare opcodes — they short-circuit via
jumps (§5.1).

**Output (phase 1)**
```
Echo      { src }     host.write_out(conv::to_string_bytes(deref(src)))
EchoConst { k }       host.write_out(conv::to_string_bytes(&consts[k]))   (inline HTML / string literals)
Print     { dst, src} echo deref(src); dst = Int(1)                       (`print` is an expression)
```

**Jumps / branches (phase 3)** — no runtime level on break/continue; the
compiler resolves them to a direct `Jmp` to the loop exit/latch (crossing a
finally routes through §3.5).
```
Jmp        { target }
JmpTrue    { cond, target }    if conv::to_bool(deref(cond))
JmpFalse   { cond, target }    if !conv::to_bool(deref(cond))
JmpNull    { src, target }     if deref(src) is Null          (for ?? / nullsafe)
JmpNotNull { src, target }
AssertCheck { dst, target }    if zend.assertions <= 0: dst = true, jump (ZEND_ASSERT_CHECK)
```
`assert(...)` (zend_compile_assert): `AssertCheck` jumps over the call when
assertions are off at run time; with zend.assertions=-1 the compiler emits
only `true`. A one-argument call gets `assert(<expr>)` as its description,
printed by `ast_export.rs`, a port of zend_ast_export over the CST.
`switch` → chain of `Cmp Eq; JmpTrue` (loose). `match` → `Cmp Identical; JmpTrue`
(strict) ending in `ThrowMatchError`.

**Arrays (phase 4)** — build into a temp, then `Assign`/`Mov`; writes use an
lvalue-cursor stack so no long-lived borrow is held into a COW array.
```
NewArray   { dst, cap: u32 }          empty PhpArray (reserve cap)
ArrPush    { arr, val }               arr[] = deref(val)        (PhpArray::push; make_mut COW on the `arr` slot)
ArrSet     { arr, key, val }          arr[ArrayKey::from_value(key)] = deref(val) ?  (TypeError on illegal offset)
ArrSpread  { arr, src }               merge: int keys renumber (push), string keys overwrite
DimFetchR  { dst, arr, key }          dst = deref(arr)[norm(key)] or Null (+undef-key warning); no autovivify ?
DimFetchIs { dst, arr, key }          like R but NO warning, Null if arr not array   (for ?? / nested isset)
DimFetchUnset { dst, arr, key }       element below `unset($a[$k]->p)`: missing key silent; string offset / scalar container Err ?
-- lvalue cursor (nested `$a[$i][$j]=v` without dangling COW borrows) --
LvalCv     { cv }                     push a cursor rooted at CV cv (autovivify Null→[] on descent)
LvalDim    { key }                    descend top cursor into norm(key) (create Null if missing; Err if scalar) ?
LvalAppend {}                         descend into a freshly appended element
AssignLval { src, dst }               commit deref(src) into cursor slot, bubbling make_mut up the path; pop. ref-aware at leaf.
                                      dst (when not src) gets the assignment's value: src, or the byte a string offset got (null if none)
FetchLvalRw{ dst }                    read current cursor value into dst AND keep cursor open (compound/inc-dec on `$a[$i]`)
-- isset / empty / unset --
IssetCv    { dst, cv }                Bool(!is_null(deref(cv)))           (unset≡null to isset)
IssetDim   { dst, arr, key }          Bool(present AND not null); no warning; chains via DimFetchIs
EmptyCv    { dst, cv }                Bool(!conv::to_bool(deref(cv)))
EmptyDim   { dst, arr, key }          Bool(!to_bool(element)); missing ⇒ true
UnsetCv    { cv }                     regs[cv] = Undef  (see "Undefined variables")
UnsetDim   { arr, key }               remove via make_mut; next free key not lowered (value-model §3c)
```

**By-name scope (as built).** extract(), compact(), get_defined_vars() and
`$$x` / `${expr}` / `global $$x` see a scope by name, as php's symbol table
(zend_rebuild_symbol_table) does: a function's compiled variables in CV order
(`scope::named_cv`; `$this` and the auto-globals are not among them), then
the variables created by name in `Frame::dyn_vars` (insertion-ordered); the
global scope's is the global symbol table. An include/eval frame uses its
includer's. A builtin reaches the innermost user-code frame through the
`Host::scope_*` methods (crates/rhp-vm/src/scope.rs). Nothing is built
ahead: frames that never look a name up pay nothing. A direct call by name
sets `Vm::call_direct` for the builtin it runs; any other way in (`$f()`,
callbacks, call_user_func, `f(...)`) is php's ZEND_CALL_DYNAMIC, which the
scope functions refuse with "Cannot call f() dynamically"
(`Host::call_is_dynamic`). The exception is `call_user_func[_array]('name', …)`
with a string literal naming an internal function: php binds that at compile
time, so the compiler sets `CallFlags::CT_BOUND` and the callee's call is
direct.

**Undefined variables (as built).** A CV slot holds `Value::Undef` (php's
IS_UNDEF) until it is assigned and again after `unset()`; `Frame::new` starts
every register undefined (temporaries are written before they are read). In the global scope a CV is a `Ref` to the symbol-table
cell, and the cell holds `Undef` instead. There is no compile-time analysis:
as in php, each instruction decides how it fetches its operands.

- R (a plain read, php's BP_VAR_R): `Vm::reg_r` raises "Undefined variable
  $x" (the name from `cv_var_names`) and reads null. The handlers that read
  an operand that may be a CV use it: arithmetic and concat, comparisons,
  casts, `echo`, `Mov`/`Assign`, array literal elements, dimension and
  property containers and keys, dynamic names, call targets, `return`,
  `throw`, `yield`, … A temporary never warns (`undef_cv` checks
  `r < num_cvs`).
- IS (`isset`, `empty`, `??`, `??=`, the `?->` under isset): `MovIs`,
  `FetchVarVarIs` and the `*Is` fetches read quietly (`reg_get`: `clone()`
  gives null).
- W (assignment, `$a[] = v`, `global`, `static`, by-ref binding): writes
  replace `Undef`; `LvalCv` leaves the root alone until the write lands, so
  `$a[0] = $a` reads (and warns about) `$a` first. A PHP reference never
  holds `Undef`: `promote` turns it into null.
- RW (`$x .= v`, `$x++`, `$a[$k] += v`): `IncDec` and `LvalCvRw` warn, then
  the variable is null; `LvalGlobalRw` / `LvalVarVarRw` do the same for
  `$GLOBALS['x']` ("Undefined global variable") and `$$n`.
- UNSET (`unset($a[$k])`): `LvalCvUnset` warns and unsets nothing.
- A method's argument window passes variables as `MakeArgRef` (the callee
  is only known when the call runs): an undefined one stays undefined in a
  shared cell, and binding decides (`bind_undef_args`): a by-reference
  parameter creates it as null, a by-value one (or `__call`) warns. A
  dynamic call (`$f($x)`) to a function, builtin or method by name does the
  same once it has resolved the callee (`settle_dyn_undef_args`).
- A user error handler run in the middle of an lvalue write keeps the open
  cursor (`with_saved_cursor`); when the handler replaces the variable under
  an undefined key, the write goes to the container php had fetched
  (`lval_undef_key`).
- `CheckCv` warns once where php reads once but rhp fetches several times
  (a `list()` source, a `match` subject php puts in a jump table, the
  operand of a `?->` guard).
- `$GLOBALS` (`globals_array`) and a closure's static variables leave
  undefined entries out; `$GLOBALS` keeps a global that is a PHP reference
  shared with something else (not a global-scope frame's own slot) as a
  reference; `unset($GLOBALS['x'])` keeps the symbol-table entry
  (an `Undef` cell), as php keeps the bucket of an IS_INDIRECT slot.
- An include shares the including function's variables by name through cells
  (`promote_slot`), which keep `Undef`; an arrow function's implicit capture
  copies the slot with `copy_slot`, so an undefined outer variable is
  undefined inside (`use ($x)` reads it, with the warning).

**foreach** — by-value over an array iterates a COW snapshot (Rc bump into a
hidden reg) by position (`PhpArray::fetch_from(pos)`, skipping holes); a temporary source is moved
into that reg. Everything else follows php's HashTable iterators: a by-ref loop
puts an iterator on the live array (`ArrayData` iterator list, id from
`next_ht_iter_id`, the same mechanism array_walk uses), so positions survive
unset/append/splice/shift/unshift and reassignment (an array without the
iterator is walked from its internal pointer); a plain object by value or by
ref iterates its live property table through an iterator kept on the object
(`ObjectData::prop_iter_*`, moved by `remove_prop`; positions count
every declared slot, empty ones too).
`pos_reg + 1` holds a placeholder object for the handle php's
zend_object_iterator takes (Traversable and hooked objects).
`keycv` = `Reg::MAX` means "no key target".
```
FeReset    { arr_reg, pos_reg, src, end }            start; empty/non-iterable (warns) -> Jmp end
FeFetch    { arr_reg, pos_reg, keycv, valcv, end }    bind next key?/value, or Jmp end
FeResetRef { arr_cv, pos_reg, end }                   by-ref: separate + iterator on the array in arr_cv
FeFetchRef { arr_cv, pos_reg, keycv, valcv, end }     promote the next element to Ref, bind valcv to it
FeFree     { arr, pos_reg, by_ref, clear_arr, end }   ZEND_FE_FREE: drop iterator, handle, temporary
```
FeFree is emitted at the loop end and before `break N`/`continue N`, `return`
and `goto` leave the loop (only loops inside the innermost try/finally; outer
ones are left to the frame). On a jump `end` is the loop end: an exception a
destructor throws there is handled as if thrown at the loop end
(ZEND_FREE_ON_RETURN). A `break`/`continue` through a finally frees the loops
outside it once the finally ran: the jump goes to a stub after the try
statement that runs their FeFree and jumps on (php interleaves FAST_CALL and
FE_FREE the same way). A temporary is not freed on exception unwinding until a
catch frees the try's temporaries or the exception leaves the frame (php's
live ranges are not modelled). Its HashTable iterator does go:
the frame records each running loop's iterator (`Frame::fe_live`), and a catch
that frees the try's temporaries, the frame going, or the same loop starting
again drops one its loop left without `FeFree`.

**References / scope (phase 6)** — reuse `Value::Ref`, no new machinery.
```
BindRef    { dst_cv, src_cv }         `$dst =& $src`: promote src slot to Ref(rc), store Ref(rc.clone()) in both
MakeRef    { dst, cv }                materialize cv slot as Ref, place Ref(rc) in dst (for `&$x` as arg/element)
BindGlobal { cv, name: NIdx }         `global $x`: bind cv to {main} scope slot for name, as a shared Ref
BindStatic { cv, slot: u32, init: CIdx }  `static $x=init`: init persistent slot on first exec; alias cv after
RefFuncResult { src }                 `=& f()`: notice when the call's result is no reference (then assigned)
FetchListW { dst, src, key }          `[&$a] = $src`: element `key` of the container `src` references, as a reference
RetRef     { src, kind }              by-ref return; `kind` (RET_REF_*) says when php notices
SetPendingRetRef { src, kind }        RetRef crossing finally: type checked now and again after it, notice at the end
ArgSpreadRef { arr, src, sig }        `f(...$a)` for by-ref params: their elements become references in `$a`
```
The value-model's §4 ("Non-variables where php wants a reference") has the
rules these follow.

**Calls / return (phase 5)** — positional args evaluated into a contiguous temp
window `[base, base+argc)`; for a builtin the VM passes `&regs[base..base+argc]`
straight to `NativeFn`. `cache` reserved for monomorphic function-resolution ICs.
(Now: a builtin's arity and by-ref positions come from php's stubs
(`rhp_bytecode::signatures`), and before the call `crates/rhp-vm/src/zpp.rs`
parses its stub-typed scalar and array arguments as zpp does, in the caller's
`strict_types` mode, so the body gets coerced values. `gen signatures` also
reads each function's C body for the parameters php parses as any value, a
resource or an open stream, so the VM checks those too and parses the
scalars after them. The VM raises a `#[\Deprecated]` builtin's deprecation
before any of this, from the stub's `since` and `message`.)

**Attributes the engine acts on.** The compiler runs php's internal
attribute table (`rhp-compiler/src/attrs.rs`, zend_attributes.c): allowed
targets, repetition and the validators, with `#[\DelayedTargetValidation]`
leaving the error to `ReflectionAttribute::newInstance()`
(`AttributeProto::validation_error`). `newInstance()` runs the
constructor from a frame that only stands for the attribute's file, line
and `strict_types` (`ProtoFlags::CALL_SITE`, zend_get_attribute_object's
dummy frame): traces show it as the call site, never as a frame. `#[\Deprecated]` and `#[\NoDiscard]`
set `ProtoFlags::DEPRECATED` / `NODISCARD`; `bind_frame` tests the two bits
once per call and, only when one is set, raises the deprecation and the
unused-result warning before the frame is entered
(`rhp-vm/src/attributes.rs`, zend_execute.c's zend_deprecated_function /
zend_nodiscard_function). "Unused" is php's zend_do_free rule: the compiler
records in `FunctionProto::discard_calls` the pcs of calls compiled last in
a freed expression (an expression statement, a `for` list entry), and the
VM looks the caller's pc up there. The message suffix comes from building the
attribute object (its constructor checks the arguments). Class constants and
enum cases carry `ClassConst::deprecated`; attributed global constants are
declared by `DeclareAttrConst { name, src, attrs }`, which keeps
`FunctionProto::const_attrs[attrs]` for the constant (fetch deprecation,
`ReflectionConstant::getAttributes()`). A trait's `#[\Deprecated]` is raised
when a class using it is linked. Trace arguments of `#[\SensitiveParameter]`
parameters (user ones, and builtins' through the stub flag `SENS`) become
`SensitiveParameterValue` objects.
```
CallBuiltin { dst, bid, base, argc }              arity-check vs Builtin; dst = (builtin.func)(self, &regs[base..+argc]) ?
CallUser    { dst, fnidx, base, argc, cache }     push frame for functions[fnidx]; bind params; dst = return value
CallByName  { dst, name: NIdx, base, argc, cache} resolve name at runtime (Err if undefined), then as CallUser/Builtin
CallDyn     { dst, callee, base, argc }           callee is a runtime callable (Closure/"fn" string/[obj,method]); resolve+dispatch
InitDynCall { callee }                            INIT_DYNAMIC_CALL's checks before the arguments run (throws what the call would)
-- InitDynCall is emitted only when an argument is more than a literal or
   a plain variable (otherwise running the arguments first can't be seen);
   a call by name to an undefined function still runs its arguments first
   (php's INIT_FCALL_BY_NAME throws before them)
Ret         { src }                               return deref(src); run pending finallys (§3.5); pop frame
RetNull     {}                                    return Null
-- argument temporaries move into the callee as they are bound (and a builtin
   takes them), so an object passed as a temporary is freed when the call
   returns and its handle comes back as in php (oop-model.md, "Fresh id")
-- php's FREE_OP, approximated: an instruction that consumes a temporary
   holding an object or array (binary operators and concatenation past
   their scalar fast paths, `(string)`, property and dimension reads)
   clears it (`Vm::free_tmp`), so its destructor runs before the next
   instruction, reported at this one; a compound assignment's operands go
   after its store, as php frees OP_DATA last (`AssignLval` frees them; a
   variable's `Assign` runs inside the operator)
-- php's live ranges, approximated: an exception caught by a try frees the
   try body's temporaries (`ExRegion::temps` up), and one leaving a frame
   frees the frame's, their destructors running (chained) before the
   handler; a failed `new`'s evaluated arguments go before its object
-- arg-staging refinements, emitted before the Call* as needed --
SendRef    { slot, cv }               place MakeRef(cv) into the arg window (by-ref parameter)
ArgName    { slot, name: NIdx }       tag a window slot named (CallUser rebinds by ParamInfo.name)
SpreadArgs { src }                    dynamic argc: switch to the *_V path (CallUserV{dst,fnidx,argsvec}) materializing Vec<Value>
```
Param binding in `CallUser` is driven by `ParamInfo` (by-value clone / by-ref
bind to the passed `Ref` / variadic collects window tail into a `PhpArray` /
default `Const` copy or `Init` block; missing+no-default ⇒ `ArgumentCountError`).
An `Init` block is guarded by `JmpIfArg { index, target }`; for a typed
parameter it ends in `RecvInitCheck { index }`, php's RECV_INIT check of the
evaluated default (a constant default was checked, and an int one widened
for a float type, at compile time).
A call with an unpack or a named argument marshals its arguments into an
array (int keys positional, string keys named) and uses the `_V` path
(`CallV`, `CallMethodV`, `NewV`, …; `CallParentHook` with `argc ==
PACKED_ARGS`): the VM places the names with the callee's `proto.params`
(`place_named_args`) or a builtin's stub arginfo (`place_builtin_named_args`).
The compiler only resolves by-reference parameters by name, for a callee it
knows. For a callee only the run time knows, a positional argument before
the unpack that is an element / property (SEND_FUNC_ARG), or a variable a
later argument may change (SEND_VAR_EX), asks the callee (`*ArgIsRef`) as it
is sent and goes into the array as a reference or a value; any other
variable goes in as a reference that a by-value parameter dereferences.
The fixed-arity register-window path is the fast path.

**Exceptions (phase 3+/5)** — see §3.4–3.5. `try` *entry* needs no opcode (the
`exception_table` describes protected ranges); only these are explicit:
```
Throw           { src }               begin unwinding with deref(src) as the in-flight Throwable
ThrowMatchError { subject }           throw \UnhandledMatchError (message from deref(subject)); for match fall-off
ThrowError      { k }                 throw \Error with the constant message k (a form php compiles but rejects when run)
CheckThis       { cv }                ZEND_FETCH_THIS: "Using $this when not in object context" unless cv holds an object
FinallyEnd      {}                    pop Frame.pending and perform it (resume after a finally); §3.5
```
`CheckThis` precedes a read of `$this` wherever the object is not
guaranteed: everywhere but instance methods and hooks (closures, static
methods, functions, top-level code). Compile-time errors (php's
E_COMPILE_ERROR) are recorded with `FileCtx::fatal` or returned with
`compile_error`; the earliest in the file wins, as php stops at the first.
Catch *matching* is ordinary code at the landing pad (the unwinder puts the
thrown value in the catch var reg; the pad does class tests — phase 7 — or
rethrows).

**Misc (phases 2, 5, 7-stub)**
```
FetchConst { dst, name: NIdx }        read a user/engine constant (Err "Undefined constant" if absent); true/false/null use LoadBool/LoadNull
DeclareFn  { fnidx }                  install functions[fnidx] into the live table (conditional/non-hoisted declarations)
MagicConst { dst, which: u8 }         __LINE__/__FILE__/__FUNCTION__/… from frame/proto
Halt       {}                         __halt_compiler(): stop this proto as top-level return
                                      (__COMPILER_HALT_OFFSET__ is folded to the file's offset at compile time)
Refuse     { feature: NIdx, reason: NIdx }  emitted ONLY for a structurally-refused node reachable at runtime (§4); raises Thrown::Unsupported
```
OOP opcodes (`New`, `FetchProp`/`AssignProp`, `CallMethod`, `CallStatic`,
`InstanceOf`, `Clone`, late static binding) are **phase 7** — same encoding and
calling convention; omitted here.

### 2.5 Inline caches, constants, names

(Not built: there are no inline caches. Method and property lookups probe
the flattened class table's hash maps; see the note at the top. Calls by
name have a per-VM resolution cache instead, §2.6.)

- `caches: Vec<InlineCache>` is a Zend-style **side** table (not interleaved),
  `Cell`-based (single-threaded ⇒ no atomics), **monomorphic to start**, guarded
  by `ClassId` (u32) compare. Reserve the `cache` field on `CallUser`/`CallByName`
  from phase 5 so the ISA shape never changes; property/method sites land in
  phase 7. Miss → ordinary resolver (same code as the oracle), which fills the cache.
- `consts` holds every literal that isn't a small inline immediate (strings /
  inline-HTML bytes as `Value::Str`, floats, big ints, constant defaults), deduped.
- `names` interns identifier bytes used at runtime by `NIdx` (var names for
  `global`/diagnostics, const names for `FetchConst`, function names for
  `CallByName`, param names for named args). Functions are case-insensitive (the
  builtin table lowercases). Engine constants come from `builtins::constants()`
  at link time; `FetchConst` consults the engine/user constant table, not `consts`.

### 2.6 Compile-time flags and call resolution

What the VM needs to know about code it runs is decided when it is
compiled, not guessed from debug info:

- `FunctionProto::flags` (`ProtoFlags`). `INTERNAL` marks every proto
  compiled from the prelude (and the VM's own `[internal function]`
  `{main}`). php runs internal code without a user frame, so the VM skips
  such frames when it reports an error's file and line or a caller's
  `strict_types`, and words their argument errors like an internal
  function's. `SHUTDOWN_SCOPE` is set only by the VM, on the copy of the
  finished script's `{main}` that holds the globals while shutdown
  callbacks run; it is no user frame either (`Vm::is_internal_frame`).
- `Call` carries `CallFlags`, php's compile-time decisions for a call
  that reaches a builtin: `INLINE` (zend_try_compile_special_func: no
  trace frame), `ICALL` (php 8.4 frameless internal calls) and `USER_CALL`
  (`call_user_func[_array]` compiled to ZEND_INIT_USER_CALL). The compiler
  sets them from the call's name and argument count: `INLINE` and
  `USER_CALL` only for a name resolved at compile time (no namespace
  fallback), `ICALL` also for a namespaced call that falls back to the
  builtin. `CallV` (an unpacked or named argument) has none: php compiles
  such a call as a plain internal call. `RESULT_UNUSED` marks a call that
  is a whole expression statement; a builtin asks `Host::result_unused()`
  (php's `!USED_RET()`, which `array_splice` checks).

A call by name is resolved once per call site and request, like php's
run-time cache slots: `Vm::call_sites` maps (proto address, pc) to the user
function or the builtin with its stub signature, so a repeated call does
no lowercasing, lookups or `is_refused` check. The cache is per VM, never
in the proto, so a program opcache shares between requests stays as
compiled, and it is not counted in `memory_get_usage()` (php allocates an
op_array's run-time cache in one piece, so reaching a new call site does
not move the number). Each entry holds a `Weak` to its proto, which keeps
the address from being reused while the entry exists. An include's or eval's `{main}`
is not cached: it is usually freed when it returns. An entry that a later
function declaration could change (a namespaced call that fell back to the
global name, or a builtin, since rhp still lets a declaration take a
builtin's name) records `Vm::fn_gen` and is resolved again after a
declaration; php would keep the first result.

The engine's helpers behind the prelude classes are builtins named `__…`
(`calls::is_helper_function`). php has no such functions, so a direct call
from prelude code reaches them and nothing else does: user calls,
callables, first-class callables, `function_exists`, `is_callable`,
`get_defined_functions` and Reflection all treat them as undefined, and a
user function may take such a name. A first-class callable of such a name
is looked up when it is made, as in php: in a namespace `MakeFcc` carries
the namespaced name, which is tried first and named in the error.

Every first-class callable resolves its callee when `MakeFcc` runs and
raises that call's errors (undefined function, class or method, a private
or non-static method, a value that is not callable); a Closure is its own
first-class callable. An unqualified name in a namespace takes the global
function when one exists (like php's per-site cache once it has found it),
else the namespaced one. In a constant expression (`const_expr`) a
`__callStatic` callable is refused. With `static_call` (static-call
syntax: `C::m(...)` or a `'C::m'` literal) `self`/`parent`/`static` name
the scope's classes and an instance method of `$this`'s class binds
`$this`; an array callable such as `['C', 'm']` gets neither.

---

## 3. The VM (`rhp-vm`)

> **Source layout (2026-11).** `crates/rhp-vm/src/lib.rs` holds the `Vm`
> struct and `Frame`; the rest is split by concern: `dispatch` (the run loop,
> one `match` over `Instr`), `calls`, `props`, `types`, `lvalue`,
> `generators`, `include`, `errors` (diagnostics, traces, unwinding),
> `lifecycle` (`Vm::new`, globals, shutdown), `host` (the `Host` impl) and
> `streams` (the `StreamTable`), beside `classes`, `compare`, `output`,
> `request`, `reflect`, `memory` and `opcache`. The sketches below predate it;
> §3.6 and §3.7 are current.

### 3.1 Vm struct

```rust
pub struct Vm {
    out: Vec<u8>,                               // the single output sink; SAPI flushes it
    stack: Vec<Frame>,                          // explicit PHP call stack (§3.3), NOT native recursion
    functions: Vec<Rc<FunctionProto>>,          // FnIdx-indexed, installed from the Program
    fn_names: HashMap<Vec<u8>, FnIdx>,          // lowercase name → FnIdx (case-insensitive)
    constants: HashMap<Vec<u8>, Value>,         // engine+user constants, seeded from builtins::constants()
    // classes: Vec<ClassEntry>, class_ids: HashMap<...>   — phase 7
    max_depth: usize,                           // recursion guard → catchable Error, not a native overflow
}

impl Host for Vm { fn write_out(&mut self, bytes: &[u8]) { self.out.extend_from_slice(bytes); } }

impl Vm {
    pub fn new() -> Self { /* seed constants from builtins::constants(); max_depth = 1<<16 */ }
    pub fn output(&self) -> &[u8] { &self.out }

    /// Execute a compiled program; returns the process exit code.
    pub fn run(&mut self, prog: &Program) -> Result<i32, Thrown> {
        self.install(prog);                         // register functions/classes
        match self.call_main(prog.main.clone()) {   // enter {main} as frame 0 and drive (§3.2)
            Ok(_)                   => Ok(0),
            Err(Thrown::Exit(code)) => Ok(code),     // exit/die: output kept, clean code
            Err(other)              => Err(other),   // uncaught throw / refusal → SAPI
        }
    }
}
```

Constant/function/class tables live **in the `Vm`**, never in a `Value`.
`Value::Object` carries a `ClassId` (u32 index), so values stay `'static`.

### 3.2 Frame

```rust
pub struct Frame {
    pub proto: Rc<FunctionProto>,   // Rc so recursion/closures share one copy
    pub regs: Vec<Value>,           // CVs [0,num_cvs), temps above, len = num_registers; a slot may be Value::Ref
    pub pc: usize,
    pub ret_dst: Reg,               // caller register the return value lands in
    pub pending: Option<FinallyCont>, // finally bookkeeping (§3.5)
}
```

### 3.3 Explicit call stack + dispatch skeleton

Explicit `Vec<Frame>` (not native recursion): deep PHP recursion raises a
catchable `Error` via the `max_depth` length check instead of aborting the
process; a suspended generator (phase, reserved) is just a detached `Frame`; and
it is the natural interop boundary for JIT-ted frames. A `CallUser` **pushes** a
frame and lets the single top-level loop continue; `Ret` pops. The only native
recursion is the outer driver restarting the loop after an exception unwinds past
a frame (§3.4), bounded by `try` nesting, not PHP call depth.

```rust
impl Vm {
    fn run_top(&mut self) -> Result<Value, Thrown> {
        loop {
            let top   = self.stack.len() - 1;
            let proto = self.stack[top].proto.clone();     // clone the Rc handle, not the slice
            let pc    = self.stack[top].pc;
            let instr = proto.code[pc];
            self.stack[top].pc = pc + 1;                   // default: fall through
            match instr {
                Instr::LoadConst { dst, k } => { let v = proto.consts[k as usize].clone(); self.set(top, dst, v); }
                Instr::Mov { dst, src }     => { let v = self.get(top, src); self.set(top, dst, v); }
                Instr::Add { dst, a, b }    => { let (x,y)=(self.get(top,a),self.get(top,b)); let v=ops::add(&x,&y)?; self.set(top,dst,v); }
                Instr::Concat { dst, a, b } => { let (x,y)=(self.get(top,a),self.get(top,b)); self.set(top,dst,ops::concat(&x,&y)); }
                Instr::Cmp { dst, a, b, kind } => { let (x,y)=(self.get(top,a),self.get(top,b)); self.set(top,dst,eval_cmp(kind,&x,&y)); }
                Instr::Echo { src }         => { let v=self.get(top,src); self.write_out(&conv::to_string_bytes(&v)); }
                Instr::Jmp { target }       => self.stack[top].pc = target as usize,
                Instr::JmpFalse { cond, target } => if !conv::to_bool(&self.get(top,cond)) { self.stack[top].pc = target as usize; },
                Instr::CallUser { .. }      => self.enter_user_call(top, instr)?,   // push a Frame; loop continues on the callee
                Instr::CallBuiltin { dst, bid, base, argc } => {
                    let args = self.collect_args(top, base, argc);
                    let out = (self.builtin(bid).func)(self, &args)?;              // self → &mut dyn Host
                    self.set(top, dst, out);
                }
                Instr::Ret { src } => {
                    self.run_pending_finallys(top)?;
                    let v = self.get(top, src);
                    let frame = self.stack.pop().unwrap();
                    match self.stack.last_mut() { Some(c) => c.deliver_return(frame.ret_dst, v), None => return Ok(v) }
                }
                Instr::Throw { src }  => { let v=self.get(top,src); return Err(self.make_throwable(v)); }
                Instr::Refuse { feature, reason } => return Err(Thrown::unsupported(self.name(feature), self.static_reason(reason))),
                // … one arm per opcode …
            }
        }
    }
}
```

`get`/`set` are register accessors on `self.stack[top].regs`: **`get` auto-derefs
`Value::Ref` on read; `set` writes *through* a `Ref`-typed slot** (`*cell.borrow_mut()=v`)
rather than overwriting it (plain slots are replaced wholesale). Opcodes fetch
operands by cloning out of the register file (an `Rc` bump, not a deep copy)
*before* calling `ops::`/builtins, side-stepping the borrow conflict between a
register borrow and `&mut self`.

### 3.4 Exception-table unwinding

```rust
pub struct ExRegion { pub start: PC, pub end: PC, pub catches: Vec<CatchArm>, pub finally: Option<PC> } // protects [start,end)
pub struct CatchArm { pub classes: Vec<PhpStr>, pub var_reg: Option<Reg>, pub target: PC }              // catch(A|B $e)
```

Both real throws and refusals ride `Err(Thrown)`, so Rust `?` does per-opcode
unwinding for free; cross-frame unwinding is driven by the table, not by scanning
instructions. The driver that owns `run_top`:

```rust
impl Vm {
    fn run_driver(&mut self) -> Result<Value, Thrown> {
        loop {
            match self.run_top() {
                Ok(v)  => return Ok(v),
                Err(t) => match self.unwind(t)? { Resume::Caught => continue, Resume::Exit(c) => return Err(Thrown::Exit(c)) },
            }
        }
    }
    fn unwind(&mut self, thrown: Thrown) -> Result<Resume, Thrown> {
        // Exit / Unsupported are NOT catchable: clear the stack, hand straight to the SAPI.
        if !is_catchable(&thrown) { self.stack.clear(); return Err(thrown); }
        loop {
            let Some(frame) = self.stack.last_mut() else { return Err(thrown); }; // escaped {main} → uncaught fatal
            if let Some(region) = find_region(&frame.proto, frame.pc) {
                if let Some(arm) = region.matching_catch(class_of(&thrown), &self.classes) {
                    if let Some(r) = arm.var_reg { frame.regs[r as usize] = thrown_to_value(&thrown); }
                    frame.pc = arm.target as usize; return Ok(Resume::Caught);   // enter catch body
                }
                if let Some(fin) = region.finally {                              // no catch matched but finally guards
                    frame.pending = Some(FinallyCont::Rethrow(thrown.clone()));
                    frame.pc = fin as usize; return Ok(Resume::Caught);
                }
            }
            self.stack.pop();   // no handler here: pop and continue unwinding in the caller
        }
    }
}
```

- **Catchable = `Thrown::Error{class,..}` only.** Matching is an
  `instanceof`-style check against the built-in exception hierarchy by class name
  until phase-7 objects carry the real class.
- **`Thrown::Exit(code)`** is not an error and not catchable: `unwind` clears the
  stack, `run` maps it to the exit code, **output is kept** (never discarded).
- **`Thrown::Unsupported{feature,reason}`** is not catchable: it unwinds every
  frame to the SAPI, which renders it with the `RHP_UNSUPPORTED` marker —
  distinct from PHP's "undefined function", and **never swallowable by a user
  `try/catch`.**

### 3.5 `finally` (one model)

A `finally` block is compiled **once**; every way of leaving the protected region
routes through it carrying what to do after, via `Frame::pending`:

```rust
pub enum FinallyCont {
    FallThrough,        // normal completion → continue after the finally
    Jump(PC),           // a break/continue crossing out
    Return(Value),      // a `return v;` crossing out — complete it after finally
    Rethrow(Thrown),    // an exception in flight — resume unwinding after finally
    ForcedClose,        // a destroyed generator runs its finally blocks, then closes
}
```

The compiler routes each exit by setting `pending` and jumping to the finally PC;
the finally region's last opcode is `FinallyEnd`, which pops `pending` and
performs it. Nested finallys chain naturally (each `FinallyEnd` that still must
cross another finally re-enters the next). A `return`/`break` *originating inside*
a finally **overrides** a pending continuation (PHP semantics: the finally's own
control flow wins) — the finally body replaces the top `pending` before
`FinallyEnd`. `flags.has_finally` lets the common finally-free function skip this
path entirely. (This unifies isa.md's FAST_CALL/FAST_RET and vm.md's
`Frame::pending`: `FinallyEnd` *is* FAST_RET; the entry jump *is* FAST_CALL.)

The walks follow zend_dispatch_try_catch_finally_helper and
zend_handle_loops_and_finally: regions are visited innermost first (the
exception table lists inner regions before outer ones). An exception
thrown inside a running finally block that no inner try handles discards
what that block was finishing; a pending exception becomes the innermost
`previous` of the new one (zend_exception_set_previous, which drops it
when it is already in the chain). A `return` or jump discards the pending
work of each finally block it leaves only after the inner finally blocks
it must run first have run (`Vm::leave_finallies`).

Generators (generators.rs) follow zend_generators.c. A suspended
generator's `Frame` lives in its `GenState`; resuming pushes it and runs
it until `Yield`/`YieldValues` (a yielded value) or `YieldFrom` (attached
to another generator) hands `yield_out` to the driver. `yield from` over
a Generator builds php's delegation tree: the delegating generator is a
child of the one it yields from, values and resumption go to the root,
and `Vm::gen_resume` moves between nodes in a loop (no Rust recursion per
level; roots and leaves cache each other as php's `node.ptr` does). A
generator whose function has finally blocks is destructible: its
destructor runs the finally blocks around its `yield` with
`FinallyCont::ForcedClose`, where a `yield` throws. Traces show a resumed
generator as called from `[internal function]` when a `Generator` method
(a `NativeCall` with `method_of`) resumed it, followed by the generators
of the `yield from` chain it runs for (`Vm::gen_resumes`).

### 3.6 Units: one copy of the bytecode

The VM never copies or patches a proto after compile: a `Program` the
compiled-script cache keeps runs as it is in every request. Three
instructions name things by a *file-local* index: `MakeClosure` and
`DeclareFunc` (the file's `Program::closures`) and `DeclareClass` (its class
declarations). Each install of a program (the main script, which is unit
0, every `include`/`require`, every `eval`) appends its closures and class
declarations to the VM's flat tables (`Vm::closures`, `Vm::decl_classes`)
and records where they start in a `UnitBase` in `Vm::units`. A frame
carries its unit, and those three instructions add the unit's base to
their operand. A program loaded right after one that added nothing to
either table has the same base, so it shares that unit: a file without
closures or classes included in a loop does not grow `Vm::units`.

Whatever can start a frame records its unit beside its proto: the function
and closure tables and the class initializer thunks hold a `UnitFn`, a
`ResolvedMethod` has `unit` (a trait method keeps the trait's), and so do
declaration slots and `ClassEntry`s. Nested thunks (parameter defaults,
attribute arguments) run in their owner's unit; for a trait's property or
constant that is the trait's. Function statics are keyed by (proto, unit,
class, name), so a closure from a file included twice keeps two sets, as
two compiles would. An include's or eval's `{main}` keeps its statics only
while it runs (`Vm::include_statics`, keyed by stack index), since php
starts them fresh on every load; so a unit shared by two loads never mixes
their statics, and the protos keyed in `Vm::statics` (functions, closures,
methods) all stay alive until the request ends.

`run_frames` takes the running frame's proto (an `Rc` clone) only when a
call or a return changes the top frame, not per instruction.

### 3.7 Sharing compiled code across threads (design notes)

Each thread compiles and caches its own programs (opcache's cache and the
compiler's prelude cache, keyed on the prelude source, are `thread_local!`)
because compiled code holds
`Rc`s. Since the VM no longer writes into protos (§3.6), what is left for
sharing one `Program` between threads is in the data types:

- **Proto links.** `Program`, `MethodProto`, `PropDefaultProto::Init`,
  `ConstInitProto::Thunk` and `ParamInfo::default_thunk` hold
  `Rc<FunctionProto>` / `Rc<ClassProto>`; they become `Arc`. The VM clones
  these handles when it installs or links (function, closure and method
  tables, `ClassEntry::proto`), which stays cheap. `Frame::proto` would make
  every call and return an atomic increment and decrement; a frame could
  borrow the proto instead, kept alive by an `Arc<Program>` held by its
  unit.
- **Constants and names.** `FunctionProto::consts` is a `Vec<Value>`, and
  `names`, `cv_var_names`, `superglobals`, `debug.file`, the class protos'
  names, `TypeHint`, `DeclMeta` and attribute arguments hold `PhpStr`
  (constant arrays hold `PhpArray`): non-atomic `Rc`s, so not `Send`.
  `LoadConst` clones a constant into the frame, which writes its
  refcount. Either a shared pool gets a `Send` representation that each
  thread turns into `Value`s on first use (memory per thread, a lookup per
  proto), or strings and arrays get php's immutable flag
  (`GC_IMMUTABLE`): clone and drop skip the counter of an immutable value,
  so a shared one is never written. The second keeps `LoadConst` a plain
  clone but adds a branch to every `PhpStr`/`PhpArray` clone and drop;
  measure it on the benchmarks first.
- **`Program::compile_diags_reported`** is a `Cell<bool>`; it becomes an
  atomic.
- **Caches.** opcache's `Cache` and the compiler's `PRELUDE_CACHE` move
  from `thread_local!` behind a process-wide lock (only the `opcache`
  module touches the cache). Preloading can then run once per process; its
  declarations are still linked into each request's VM. The linked prelude
  classes `Vm::install_prelude` copies into each request are VM state, so
  they stay per thread.
- **Memory accounting.** The counters are per thread and a block freed on
  another thread is charged there, so shared programs are built and freed
  inside `rhp_values::mem::uncounted`, as the cache already does.
- **What stays per VM.** Units, the function, closure and declaration
  tables, linked `ClassEntry`s (static properties, lazily evaluated
  constants, enum case objects), statics, the call-site cache (§2.6) and
  everything else a request changes. None of it needs to be `Send`.

The model stays one VM per thread with plain `Rc` (sapi.md, *Threading
model*); `Arc` comes later, behind a flag, once measured.

---

## 4. Refusals in the new pipeline (structural, in `rhp-compiler`)

Refused constructs **never acquire an opcode**. They are rejected in the compiler
at the AST node; the JIT (later) therefore never has to reason about them.

### 4.1 Syntactic constructs — `rhp-compiler/src/refuse.rs`

The lowerer pattern-matches these nodes and, instead of emitting any `Call*`/eval
opcode, returns `Err(Thrown::unsupported(...))` at compile time (or, if the node
sits on a conditionally-reachable path the compiler chooses not to reject
eagerly, emits a single `Refuse { feature, reason }` opcode that raises the same
uncatchable `Thrown::Unsupported` when reached — never a `Throw`, so a `catch`
can't swallow it):

| PHP construct | AST node | action |
|---|---|---|
| `eval(...)` | `Construct::Eval` | **refuse — no `Eval` opcode exists, ever** |
| `include`/`require`(`_once`) | `Construct::{Include,IncludeOnce,Require,RequireOnce}` | refuse (filesystem/code-load) |
| backticks `` `…` `` | `CompositeString::ShellExecute` | compiled as a `shell_exec()` call with php 8.5's deprecation (no longer refused) |
| `(real)` cast | `UnaryPrefixOperator::RealCast` | refuse (`(unset)` is php's compile error; `(void)` compiles, §2) |

### 4.2 Function-name refusals — `is_refused()`

`create_function`, the string form of `assert`,
`exec`/`system`/`shell_exec`/`passthru`/`proc_open`/`popen`: no dedicated AST
node. For a **statically-named** call the compiler checks
`builtins::is_refused(name)` at call-lowering time and emits the refusal in place
of the `CallBuiltin`; for a **dynamically-named** call (`CallByName`/`CallDyn`)
the VM checks `is_refused` at resolution time and raises the same uncatchable
`Unsupported`. Either way the refusal is `Unsupported`, not a catchable `Error`.

> Load-bearing, per the project's explicit instruction that `eval` in particular
> is **not to be implemented**: it is absent from the opcode set by construction.
> The `eval_* → walk_*` oracle rename (reorg.md §5) is cosmetic — the
> `Construct::Eval` arm stays refused in the oracle too.

---

## 5. Lowering patterns (how the AST becomes bytecode)

`tN` = temp register, `cv(x)` = the CV for `$x`.

### 5.1 Expressions / short-circuit
```
echo 1 + 2;              LoadInt t0,1 ; LoadInt t1,2 ; Add t0,t0,t1 ; Echo t0
$x = 5; $x += 3;         LoadInt t0,5 ; Assign cv(x),t0 ; Add t0,cv(x),<k 3> ; Assign cv(x),t0
$a && $b → t0:           JmpFalse cv(a),Lf ; JmpFalse cv(b),Lf ; LoadBool t0,true ; Jmp Le ; Lf: LoadBool t0,false ; Le:
$x ?? $y → t0:           Mov t0,cv(x) (or DimFetchIs) ; JmpNotNull t0,Le ; Mov t0,cv(y) ; Le:
```

### 5.2 Control flow
```
while(C)B:   Ltop: <C→t0> ; JmpFalse t0,Lend ; <B: break→Jmp Lend, continue→Jmp Ltop> ; Jmp Ltop ; Lend:
```
`for`/`do-while` same shape (the `for` condition is the last expr of its
conditions list; others run for side effects). `switch` → `Cmp Eq; JmpTrue
case_i` + fall-through + `break→Jmp Lend`. `match` → `Cmp Identical; JmpTrue
arm_i` ending in `ThrowMatchError subject`. `break 2;` across two loops → `Jmp
<outer Lend>` (resolved at compile time; crossing a finally routes via §3.5).

### 5.3 Arrays
```
$a=[1,'k'=>2,...$b];     NewArray t0,3 ; LoadInt t1,1 ; ArrPush t0,t1 ; LoadInt t1,2 ; ArrSet t0,<k "k">,t1 ; ArrSpread t0,cv(b) ; Assign cv(a),t0
$a[$i][$j]=$v;           <i→t0> <j→t1> ; LvalCv cv(a) ; LvalDim t0 ; LvalDim t1 ; AssignLval cv(v)
$a[$i]+=1;               <i→t0> ; LvalCv cv(a) ; LvalDim t0 ; FetchLvalRw t1 ; Add t1,t1,<k 1> ; AssignLval t1
```

### 5.4 Functions / args / try-catch-finally / refs
```
strlen($s):              Mov r_base,cv(s) ; CallBuiltin t0,bid(strlen),r_base,1
foo($x,...$rest):        Mov r_base,cv(x) ; SpreadArgs cv(rest) ; CallUserV t0,fnidx(foo),<argsvec>
sort($arr) (by-ref p0):  SendRef r_base,cv(arr) ; CallUser t0,fnidx(sort),r_base,1
return $x; / return;     Ret cv(x) / RetNull
$b =& $a;                BindRef cv(b),cv(a)        global $g;   BindGlobal cv(g),name(g)

try/catch/finally (exception_table: {[Ltry,Lend)->catch Lcatch, finally Lfin}):
  Ltry:   <try body>
          (pending=FallThrough) Jmp Lfin                ; normal completion runs finally
  Lcatch: <type test; no match → Throw r_exc> ; Assign cv(e),r_exc ; <catch body> ; (pending=FallThrough) Jmp Lfin
  Lfin:   <finally body> ; FinallyEnd                   ; resume per pending
  Lafter:
```

### 5.5 PHP 8.5 forms
```
$x |> f(...):            <x→t0> ; (Mov t1,cv(x) for a variable) ; Call f with t0/t1 as its only argument
$x |> $o->m(...):        <x→t0> ; <o→t1> ; CallMethod t1,m (t0)       (C::m(...) likewise: CallStatic)
$x |> <expr>:            <x→t0> ; <expr→t1> ; CallDyn t1 (t0)
clone $x / clone($x):    CloneObj t0,cv(x)                           (ZEND_CLONE)
clone($x, $w):           Call clone (the builtin: __clone, then $w written from the caller's scope)
new C($a):               NewObj t0,C ; <args> ; NewCall t0,window     (no arguments: New)
```
The pipe compiles like zend_compile_pipe: the operand first, copied out of
a variable so a by-reference parameter refuses it; an FCC on the right is
called directly (no Closure, no extra frame). The compiler stands a
synthetic node for each computed register (`ProtoBuilder::expr_regs`) and
reuses the ordinary call lowering. `NewObj` is php's ZEND_NEW: the class
checks run and the object exists before the arguments, so nested `new`s
get php's object ids; an exception in between releases the object
without its destructor (`FunctionProto::new_ranges`).

---

## 6. Differential harness

> **Changed since this spec was written.** The original plan diffed three
> engines (php ⟷ vm ⟷ the tree-walking oracle) through a shared `Engine`
> trait, with the oracle as a php-free cross-check on the subset it covered.
> The VM went on to hold every golden by itself, and the oracle was removed
> (2026-11). This section describes the harness as it is now.

- **Goldens** (`tests/golden/**/NAME.php` + `NAME.out`). `NAME.out` is the exact
  stdout the reference `php` printed for the fixture.
  `crates/rhp-cli/tests/diff.rs` (`vm_matches_php_baseline`) compiles every
  fixture, runs it on the VM the way the CLI does (include/eval via the CLI
  includer; `run_main`'s uncaught-throwable report, shutdown functions and
  output flush), and requires byte-identical output. It needs no `php` binary,
  so it also runs in the "test (no php)" CI job.
- **CLI fixtures** (`tests/cli`, `crates/rhp-cli/tests/cli.rs`): the built
  `rhp` against php's committed stdout, stderr and exit status for argv,
  stdin, `-d`/`-r`, uncaught errors and `fork`.
- **Live php diffs.** The built `rhp` and a real `php` / `php-cgi` run the same
  input and their outputs are compared byte for byte: rhp-cli's live tests
  (CGI fixtures, memory_limit, intl, phar; they skip when no `php` is on
  `PATH`), and `rhp-difftest` (the php-src `.phpt` corpus with its
  known-passing ratchet in `tests/phpt/`, fixture apps, extension sweeps).
- **Refusals** have no php output to compare against, so their tests
  (`crates/rhp-cli/tests/unsupported.rs`) assert the `RHP_UNSUPPORTED` marker
  instead.

---

## 7. Migration + build checklist (cargo green at every step)

Each step ends with `cargo build --workspace && cargo test --workspace` green.
Steps 1–4 are pure refactors (the runner doesn't exist yet, so goldens are
unaffected); 5–10 build the VM path and wire the runner.

0. **Baseline.** Build green (one known dead-code warning on `ops::intdiv`).
   Snapshot current tree-walker output for a couple of goldens as a regression anchor.
1. **Move `Thrown` → `rhp-values`.** New `rhp-values/src/thrown.rs`;
   `pub mod thrown; pub use thrown::Thrown;`. In `rhp-builtins` delete the
   enum, add `pub use rhp_values::Thrown;`. No other crate changes.
2. **Create `rhp-semantics`, move `ops`.** New crate (dep `rhp-values`);
   move `rhp-runtime/src/ops.rs`; rewrite `use rhp_builtins::Thrown` →
   `use rhp_values::Thrown`; re-export. In `rhp-runtime` replace `mod ops;`
   with `use rhp_semantics::ops;`. Expose `intdiv` (the VM will use it).
3. **Lift shared helpers into `rhp-semantics`.** Move `deref`,
   `inc_dec_value`, `cast_to_array` out of the tree-walker as `pub fn`s; update
   callers. (Prevents the VM re-implementing them in step 8.)
4. **Rename `rhp-runtime` → `rhp-oracle`; apply `walk_*`.** Rename dir +
   `Cargo.toml name`; update `rhp-cli` dep. Apply the reorg.md §5 renames
   (`eval_* → walk_*`); the `Construct::Eval` refusal arm stays. Add the
   `rhp-builtins → rhp-semantics` dep so `max/min` can use
   `ops::loose_compare`.
5. **Stand up `rhp-bytecode` (ISA only).** New crate (dep `rhp-values`):
   `Instr`, `FunctionProto`, `Program`, `ExRegion`/`CatchArm`, the newtypes,
   `InlineCache`. Data only, no executing methods. Nothing depends on it yet.
6. **`rhp-compiler` — Phase-1 subset.** New crate (deps `rhp-frontend`,
   `rhp-bytecode`, `rhp-values`). Lower echo/inline-HTML, literals,
   expressions (binary with short-circuit, unary), casts, builtin calls, array
   literals, variables-as-registers, assignment/compound. `refuse.rs` rejects
   `Construct::Eval`/`Include*`/backtick/`(unset)`-cast structurally (§4). Unit-test
   each refused construct → `Unsupported`.
7. **`rhp-vm` executing the subset.** New crate (deps `rhp-bytecode`,
   `rhp-semantics`, `rhp-builtins`, `rhp-values`). `Vm` + `Frame` +
   explicit `Vec<Frame>` stack; `impl Host`; `run(&Program) -> Result<i32,Thrown>`;
   dispatch delegating to `ops::`/`registry()`. Unit-test VM vs oracle on
   expressions.
8. **Wire `rhp-cli` to run the VM.** Real SAPI: read `.php`, parse, compile,
   run on the VM, write stdout, exit with the code.
9. **Green all 21 goldens under the VM.** Add `runner.rs` diffing php/vm/oracle
   (§6); iterate on compiler + VM until all 21 match php/`.out`/oracle. These are
   scalar/operator/dump heavy — all covered by shared `ops`/`conv`/`fmt`/`dump`,
   so divergences are lowering bugs. Confirm `rhp-runtime` fully removed from
   `members`.
10. **Then grow coverage** in order: (viii) control-flow opcodes +
    exception-table/finally, (ix) functions (calls/return/params/by-ref/variadic/
    named, explicit call stack), (x) arrays-full (lvalue cursor, foreach, spread).
    Re-green goldens + add fixtures at each sub-step.

---

## 8. Do-not-regret list (VM-specific)

1. **Register allocation correctness.** CVs are fixed `[0,num_cvs)` with params
   first; temporaries are a high-water bump freed at statement boundaries. A temp
   freed while still live (e.g. reused across a short-circuit join) silently
   corrupts a value — free only at true expression-statement ends, and assert
   `num_registers` is the real peak. This is the highest-frequency lowering bug.
2. **COW through registers.** Mutate an `Array`/`Str` **in the register slot**
   (`make_mut` on `regs[i]`), never on a cloned-out `Rc`. `$b=$a; $b[0]=1;` must
   not touch `$a`: `Mov` shares the `Rc`, `AssignLval`/`ArrSet` triggers COW.
3. **Deref discipline.** `get` auto-derefs `Value::Ref`; `set` writes **through**
   a `Ref` slot, never overwrites it. Every by-ref / `global` / `foreach &$v`
   test depends on this one rule.
4. **Exception-table off-by-one.** Regions protect `[start, end)` as *instruction
   indices* and are stored innermost-first; the throwing `pc` has already been
   advanced past the faulting instruction in the loop, so region membership must
   be checked against the **pre-increment** pc (capture it before `pc += 1`, or
   test `pc-1`). Get this wrong and a throw lands in the wrong (or no) handler.
5. **finally continuation precedence.** A `return`/`break`/`continue` originating
   *inside* a finally overrides any pending continuation; a plain throw must
   survive a finally that falls through. Drive it all through one `FinallyCont`
   and one `FinallyEnd` — do not special-case per exit site.
6. **Exit/Unsupported stay uncatchable.** Never lower a refusal to `Throw` (a
   user `catch` would swallow it); never let `Exit` discard the output buffer.
7. **Two float precisions survive lowering.** `Echo`/`Concat`/`(string)` use
   `fmt::float_echo` (precision-14); `var_dump`/`var_export` use
   `fmt::float_serialize` (−1). The compiler routes to the right one; the opcode
   does not choose.
8. **Stack-depth guard, not native overflow.** Check `stack.len() < max_depth` on
   every `CallUser` → catchable `Error`, never a Rust stack abort.
9. **Keep it JIT-lowerable.** Fixed-shape `Copy` operands, instruction-index PCs,
   side-table constants/caches, exception table (not `?`-only control flow), and
   one explicit terminator per basic block. Never bury an opcode's semantics in
   the loop such that a helper can't be extracted — the JIT calls the same
   `ops::`/`NativeFn` on cold paths.
10. **The goldens are the regression net.** Every `tests/golden/**/*.out` is php's
    own output, and the VM is held to all of them without needing the `php`
    binary (§6), so a VM regression fails CI even where no php is installed.
    Keep every golden green, and add a fixture alongside each fix.
