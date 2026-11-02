# rhp bytecode ISA — the register VM core

Status: design, execution core (register bytecode compiler + VM). Target
semantics: PHP 8.5 (ground truth: the reference `php`, see `crates/rhp-builtins/src/version.rs`; probed on 8.5.8). This document fixes the
**instruction set, chunk format, register model, and lowering patterns** for the
primary execution engine. The tiny tree-walker in `rhp-runtime/src/lib.rs`
survives only as a differential oracle; this ISA is what actually runs.

Read first (this doc assumes them and reuses them verbatim):

- [`value-model.md`](./value-model.md) — the owned `Value` enum, `PhpStr`/`PhpArray`/`PhpRef`, COW vs aliasing, `ArrayKey` normalization.
- [`php-semantics.md`](./php-semantics.md) — byte-exact output/number/compare/cast rules; the precision-14 vs serialize_precision-−1 split.
- [`ast-surface.md`](./ast-surface.md) — every `Statement`/`Expression`/operator node the compiler lowers from.
- `crates/rhp-runtime/src/ops.rs` — PHP operator semantics over `Value`. **The VM calls these; it never reimplements operators.**
- `crates/rhp-values/src/{lib.rs,conv.rs,fmt.rs}` — `Value`, `conv::*`, `fmt::*`.
- `crates/rhp-builtins/src/lib.rs` — `Thrown`, `Host`, `NativeFn`, `registry()`, `is_refused()`.

## 0. Design goals, in priority order

1. **Byte-exact PHP 8.5.8 behavior.** The ISA is a lowering target; semantics
   live in `ops::`/`conv::`/`fmt::` and must not drift.
2. **Debuggable now.** A human reads a disassembly and a `Debug`-printed `Instr`
   and knows exactly what runs. The dispatch loop is one exhaustive `match`.
3. **Lowerable to a Cranelift JIT later.** Every instruction is a small, typed,
   `Copy` record with RISC-like operands (register / constant / immediate — no
   memory-operand addressing modes). Lowering = a `match` over `Instr` emitting
   IR; register indices map to SSA-value lookups. We do **not** build the JIT
   now, but no decision here forecloses it.
4. **Single-threaded.** `Rc`/`RefCell` only (no `Arc`). Values are owned
   `'static`; the arena holds only the AST, which the compiler consumes and
   discards — **a `FunctionProto` carries no `'arena` borrow.**
5. **Refuse the dangerous set structurally.** `eval`, `include`/`require`,
   backticks, `(unset)` cast, and `$$x` writes **have no opcode and are rejected
   by the compiler** (§11). Function-level refusals (`extract`/`compact`/…) go
   through `builtins::is_refused()` at call-lowering time.

---

## 1. Chunk / function format

A compilation unit is one **`FunctionProto`** per PHP function, method, closure,
and one for the script body itself. Everything the VM needs to run a function is
in its proto; protos reference each other and the builtin table only by small
integer id, so a proto is self-contained, `'static`, and cheap to clone/share
(`Rc<FunctionProto>`).

```rust
use std::rc::Rc;
use rhp_values::{PhpStr, Value};

/// A compiled PHP function / method / closure / the script body.
pub struct FunctionProto {
    pub name: PhpStr,                 // "{main}" for the script body; "{closure}" for closures
    pub num_params: u16,              // declared parameter count (incl. variadic tail)
    pub params: Vec<ParamInfo>,       // len == num_params; binding metadata (by-ref/variadic/named/default)
    pub num_cvs: u16,                 // named-local registers: [0, num_cvs) are CVs (params first)
    pub num_registers: u16,           // total frame register-file size (CVs + temporaries)
    pub code: Vec<Instr>,             // the instruction stream; PC = index into this Vec
    pub constants: Vec<Value>,        // immutable literal / default pool, referenced by CIdx
    pub names: Vec<PhpStr>,           // interned identifier bytes (var/const/func/arg names), by NIdx
    pub exception_table: Vec<ExRegion>, // ordered inner→outer; see §8
    pub debug: DebugInfo,             // line map + CV names; never consulted for semantics
    pub flags: ProtoFlags,            // generator?, returns-by-ref?, has-finally?, uses-this?
}

pub struct ParamInfo {
    pub name: NIdx,                   // parameter name, for named-argument binding
    pub cv: Reg,                      // CV register the param binds (normally == positional index)
    pub by_ref: bool,                 // `&$p`: bind callee CV to the caller slot's Ref
    pub variadic: bool,              // `...$rest`: collect the remaining args into a PhpArray
    pub default: Option<Default>,     // present ⇒ optional parameter
}

pub enum Default {
    Const(CIdx),                      // a constant default (covers the vast majority)
    Init(PC),                         // non-constant default: run this init block, then fall through
}

pub struct DebugInfo {
    pub lines: Vec<LineEntry>,        // sorted; pc→source line, run-length encoded
    pub cv_names: Vec<NIdx>,          // len == num_cvs: register→variable name, for disassembly/backtraces
    pub file: PhpStr,                 // logical source name
}
pub struct LineEntry { pub pc: PC, pub line: u32 }
```

Operand index newtypes — all `Copy`, all small:

```rust
pub type Reg  = u16;   // register-file index: CVs in [0,num_cvs), temporaries above
pub type CIdx = u32;   // index into FunctionProto.constants
pub type NIdx = u32;   // index into FunctionProto.names (interned identifier bytes)
pub type PC   = u32;   // instruction index into FunctionProto.code (jump target / handler)
pub type FnIdx = u32;  // index into the engine's function table (user functions/closures)
pub type BId   = u32;  // index into the builtin table (resolved from registry() at link time)
```

### 1.1 The script is itself a proto

The whole file compiles to a `FunctionProto { name: "{main}", num_params: 0 }`.
Its CVs are the **global variables**; running the script is "call `{main}` with
no args." A top-level `return;` is `RET`/`RET_NULL` out of `{main}` and ends the
script; `exit`/`die` raise `Thrown::Exit` (uncatchable, §8). Inline HTML, `echo`,
and top-level statements lower exactly as they would inside any function. There
is no separate "top-level" execution path — the VM only ever runs protos.

### 1.2 Linking / the function table

Compilation produces a `Program { main: Rc<FunctionProto>, functions: Vec<Rc<FunctionProto>>, builtins: Vec<&'static Builtin> }`.
Unconditional top-of-scope `function`/`class` declarations are **hoisted**: the
compiler assigns each a `FnIdx` up front so forward references resolve statically
(a `CALL_USER` carries the `FnIdx`). Conditional declarations (inside `if`, etc.)
are **not** hoisted; they emit a `DECLARE_FN fn_idx` opcode that installs the
proto into the live function table when control reaches it. Calls to a name not
statically known lower to `CALL_BY_NAME` (runtime lookup) or `CALL_DYN`.

---

## 2. Register model

Modeled on Zend's **compiled variables (CVs)** plus temporaries, but the entire
frame is one flat register file.

- **One frame = one `Vec<Value>` of length `num_registers`**, all slots
  initialized to `Value::Null` on entry. (A `SmallVec`/stack-slab arena is a
  later optimization; the type contract is "a contiguous indexable slot array.")
- **CVs: `[0, num_cvs)`.** Each named local — every `$var` the function mentions,
  parameters first — gets exactly one permanent register, chosen at compile time.
  Reading `$x` is "use register `cv(x)` as a source operand"; there is **no
  load/store opcode for a direct variable** — the register *is* the variable.
- **Temporaries: `[num_cvs, num_registers)`.** SSA-ish scratch for subexpression
  results. The compiler allocates them with a simple high-water stack allocator
  and frees (reuses) them as subexpressions complete. `num_registers` is the peak.
- **Ref-aware slots.** A CV register may hold a `Value::Ref(Rc<RefCell<Value>>)`
  after `&` promotion (value-model.md §4). Temporaries never hold a live `Ref`
  that outlives the instruction producing it. "Read a variable as a value" always
  derefs one level; "write through a variable" writes into the cell if the slot
  is a `Ref`, else overwrites the slot (§7, references).

### 2.1 Addressing modes (exactly three)

| Mode | Encoding | Meaning |
|---|---|---|
| **Register** | `Reg` (u16) | a frame slot (CV or temp); the only mutable operand space |
| **Constant** | `CIdx` (u32) | an immutable `Value` in `constants` (literals, defaults) |
| **Immediate** | inline scalar in the `Instr` variant | small ints, bools, jump targets (`PC`), arg counts, `NIdx`/`BId`/`FnIdx` |

No instruction addresses memory, an array element, or a property *directly* —
those go through explicit fetch/store opcodes (§ array, §references). This
RISC-like shape is what keeps JIT lowering a straight `match`.

### 2.2 Operand encoding: fixed-width `enum Instr` (decided)

**Decision: a fixed-width `#[derive(Clone, Copy, Debug)] enum Instr`, one variant
per opcode, carrying typed `Copy` operand fields — not a byte-packed stream.**

```rust
#[derive(Clone, Copy, Debug)]
pub enum Instr {
    Add { dst: Reg, a: Reg, b: Reg },
    LoadConst { dst: Reg, k: CIdx },
    JmpFalse { cond: Reg, target: PC },
    // ... (full set in §4)
}
```

Invariant that keeps this cheap: **every operand is a small `Copy` index or
immediate; no variant owns heap data.** Names/`Value`s/defaults live in side
tables (`names`, `constants`) and are referenced by index. So `Instr: Copy`, its
size is bounded by the widest variant (≈ 16 bytes), and `code: Vec<Instr>` is a
flat, cache-friendly array.

Rationale (goals 2 and 3):

- **Debuggability now.** The dispatch loop is `match instr { … }` with the
  compiler enforcing exhaustiveness; `Debug` gives a readable disassembly for
  free; no hand-written decoder to desync from the encoder.
- **JIT later.** Cranelift lowering is the *same* `match` emitting IR instead of
  executing; typed fields map one-to-one to SSA-value/stack-slot lookups. A
  byte-packed stream would force a decoder in both the interpreter and the JIT,
  and complicate the `pc→line` map and exception-table `PC`s.
- **Cost accepted.** ~16 bytes/instr vs a ~4-byte packed form is more I-cache
  pressure, but the hot cost in this engine is `Value` clone/COW and `ops::`
  calls, not instruction fetch. If profiling ever demands density we can add a
  packed serialization *behind the same opcode set* (the enum stays the
  authoring/IR form) without touching lowering. We do not pay that complexity now.

`PC` is an **instruction index** (into `code`), not a byte offset — jumps and the
exception table stay valid regardless of variant widths.

---

## 3. How the VM reuses `ops::`, `conv::`, `fmt::`

The VM is a dispatcher, not a semantics engine. Binary/unary arithmetic, compare,
concat, and bitwise opcodes are thin wrappers that call the existing operator
functions, which already deref `Value::Ref`, coerce types, and return
`Result<Value, Thrown>`:

```rust
// in the dispatch loop (regs: &mut [Value]):
Instr::Add  { dst, a, b } => regs[dst as usize] = ops::add(&regs[a as usize], &regs[b as usize])?,
Instr::Sub  { dst, a, b } => regs[dst as usize] = ops::sub(&regs[a as usize], &regs[b as usize])?,
Instr::Div  { dst, a, b } => regs[dst as usize] = ops::div(&regs[a as usize], &regs[b as usize])?,   // may Err DivisionByZeroError
Instr::Mod  { dst, a, b } => regs[dst as usize] = ops::modulo(&regs[a as usize], &regs[b as usize])?,
Instr::Pow  { dst, a, b } => regs[dst as usize] = ops::pow(&regs[a as usize], &regs[b as usize])?,
Instr::Concat { dst, a, b } => regs[dst as usize] = ops::concat(&regs[a as usize], &regs[b as usize]),
Instr::BitAnd { dst, a, b } => regs[dst as usize] = ops::bitwise(&regs[a as usize], &regs[b as usize], ops::BitOp::And),
Instr::IsEq { dst, a, b } => regs[dst as usize] = Value::Bool(ops::loose_eq(&regs[a as usize], &regs[b as usize])),
Instr::IsIdentical { dst, a, b } => regs[dst as usize] = Value::Bool(ops::strict_eq(&regs[a as usize], &regs[b as usize])),
Instr::IsLt { dst, a, b } => regs[dst as usize] = Value::Bool(ops::loose_compare(&regs[a as usize], &regs[b as usize]).is_lt()),
Instr::Spaceship { dst, a, b } => regs[dst as usize] = ops::spaceship(&regs[a as usize], &regs[b as usize]),
Instr::Neg  { dst, a } => regs[dst as usize] = ops::negate(&regs[a as usize])?,
Instr::Echo { src } => { let b = conv::to_string_bytes(&deref(&regs[src as usize])); self.host.write_out(&b); }
```

The `?` is the entire exception integration: any `Err(Thrown)` an operator
returns enters the unwinder (§8). The `-2 ** 2 == -4`, int-overflow→float,
`0 == "a"` is false, precision-14 float echo, etc. are all already correct in
`ops::`/`conv::`/`fmt::` and need no restating here.

`deref` is the one-level reference read used wherever a *value* is needed from a
possibly-`Ref` slot (identical to the helper already in `ops.rs`/`lib.rs`):

```rust
fn deref(v: &Value) -> Value { match v { Value::Ref(r) => r.borrow().clone(), o => o.clone() } }
```

---

## 4. The opcode set

Grouped; each line is `OPCODE operands — semantics`. All `Reg` operands name
frame registers; sources are read (deref-on-use where a value is required),
`dst` is written. `?` marks opcodes that may return `Err(Thrown)` and enter the
unwinder. Phases refer to ast-surface.md §14.

### 4.1 Load / move / constants  (phase 1–2)

```
NOP                               — do nothing (padding / patched-out)
MOV         dst, src              — dst = deref(src).clone()   (value copy; used for temps and `$b = $a`-style value moves)
LOAD_CONST  dst, k               — dst = constants[k].clone()
LOAD_NULL   dst                   — dst = Null
LOAD_BOOL   dst, b:bool           — dst = Bool(b)              (true/false literals)
LOAD_INT    dst, n:i64            — dst = Int(n)               (inline small-int immediate; large ints via LOAD_CONST)
```

`MOV` derefs its source: a plain PHP assignment `$b = $a` copies the *value*, not
the reference binding (reference assignment is `BIND_REF`, §4.11). Writing into a
CV that currently holds a `Ref` is **not** `MOV` — see `ASSIGN`.

### 4.2 Variable store (ref-aware)  (phase 2, 6)

```
ASSIGN      cv, src               — write deref(src) into CV `cv`, THROUGH a Ref if the slot holds one:
                                    if regs[cv] is Ref(rc) { *rc.borrow_mut() = deref(src) } else { regs[cv] = deref(src) }
```

Reading a CV needs no opcode (the register is the operand). Compound assignment
(`$x += e`) is **not** a dedicated opcode: it lowers to `ADD tmp, cv, e` then
`ASSIGN cv, tmp`, reusing `ops::add` and the ref-aware store (array/property
compound assign uses the cursor path, §4.9).

### 4.3 Arithmetic / bitwise / unary  (phase 2) — all route to `ops::`

```
ADD   dst, a, b   — ops::add(a,b)?            SUB dst,a,b — ops::sub?         MUL dst,a,b — ops::mul?
DIV   dst, a, b   — ops::div(a,b)?            MOD dst,a,b — ops::modulo?      POW dst,a,b — ops::pow?
CONCAT dst, a, b  — ops::concat(a,b)          (no Err; always a string)
BIT_AND dst,a,b   — ops::bitwise(..And)       BIT_OR/BIT_XOR/SHL/SHR — ops::bitwise(..Or/Xor/Shl/Shr)
NEG   dst, a      — ops::negate(a)?           BIT_NOT dst,a — ops::bitwise_not(a)
NOT   dst, a      — Bool(!conv::to_bool(deref(a)))
```

### 4.4 Casts  (phase 2) — route to `conv::`

```
CAST_INT    dst, a — Int(conv::to_int(a))          CAST_FLOAT dst, a — Float(conv::to_float(a))
CAST_STRING dst, a — Str(conv::to_string_bytes(a)) CAST_BOOL  dst, a — Bool(conv::to_bool(a))
CAST_ARRAY  dst, a — (array) cast (scalar→1-elem, null→[], array→self; see tree-walker `cast_to_array`)
```

`(object)` is phase 7. `(unset)`/`(real)`/`(void)` have **no opcode** — the
compiler rejects them (§11).

### 4.5 Compare  (phase 2) — route to `ops::`

```
IS_EQ dst,a,b         — Bool(ops::loose_eq)        IS_NEQ dst,a,b       — Bool(!loose_eq)
IS_IDENTICAL dst,a,b  — Bool(ops::strict_eq)       IS_NOT_IDENTICAL     — Bool(!strict_eq)
IS_LT dst,a,b / IS_LE / IS_GT / IS_GE             — Bool(ops::loose_compare(a,b).is_lt()/is_le()/is_gt()/is_ge())
SPACESHIP dst,a,b     — ops::spaceship(a,b)        (Int -1/0/1)
```

(`&&`/`||`/`and`/`or` do **not** become compare opcodes — they short-circuit via
branches, §6.1. `xor`/`!=`-style logical booleans use `NOT`/`IS_EQ` as needed.)

### 4.6 Jumps / branches  (phase 3)

```
JMP        target                 — pc = target
JMP_TRUE   cond, target           — if conv::to_bool(deref(cond)) pc = target
JMP_FALSE  cond, target           — if !conv::to_bool(deref(cond)) pc = target
JMP_NULL   src, target            — if deref(src) is Null pc = target   (for `??` / nullsafe short-circuit)
JMP_NOT_NULL src, target          — inverse
```

`break N`/`continue N` carry **no runtime level** — the compiler knows the loop
nesting and resolves them to a direct `JMP` to the right loop exit/latch (or, if
a `finally` sits between, to a finally-leave sequence, §8.3). `switch` counts as
one level, matching PHP. Fused compare-and-branch opcodes (`JMP_EQ a,b,target`,
…) are a deliberate *later* peephole optimization for the JIT; the baseline emits
`IS_* ; JMP_TRUE/FALSE`.

### 4.7 Output  (phase 1)

```
ECHO       src                    — host.write_out(conv::to_string_bytes(deref(src)))
ECHO_CONST k                      — host.write_out(conv::to_string_bytes(&constants[k]))   (inline HTML / string literals — no reg needed)
PRINT      dst, src               — host.write_out(to_string_bytes(deref(src))); dst = Int(1)   (`print` is an expression ⇒ 1)
```

`echo $a, $b, $c;` → three `ECHO`. Inline-HTML `Statement::Inline` and bare
string literals use `ECHO_CONST` (the bytes are interned in `constants`); a
`Shebang` inline is never emitted.

### 4.8 Increment / decrement  (phase 2) — ref-aware, route to the `inc_dec_value` helper

```
PRE_INC  dst, cv   — v = inc_dec(read(cv), +1); write-through cv; dst = v
PRE_DEC  dst, cv   — v = inc_dec(read(cv), -1); write-through cv; dst = v
POST_INC dst, cv   — old = read(cv); write-through cv = inc_dec(old,+1); dst = old
POST_DEC dst, cv   — old = read(cv); write-through cv = inc_dec(old,-1); dst = old
```

`inc_dec` is the existing `inc_dec_value` (null++→1, numeric step, overflow→float;
alpha-string increment to be completed). On array elements / properties the
compiler uses the fetch-RW cursor (§4.9) + the same helper.

### 4.9 Arrays  (phase 4)

Construction (build into a temp, then `ASSIGN`/`MOV` to the target):
```
NEW_ARRAY     dst, cap:u32        — dst = empty PhpArray (reserve cap)
ARR_PUSH      arr, val            — arr[] = deref(val)   (PhpArray::push, COW via make_mut on the `arr` slot)
ARR_SET       arr, key, val       — arr[norm(key)] = deref(val)?   (ArrayKey::from_value; Err TypeError on illegal offset)
ARR_SPREAD    arr, src            — merge deref(src) array into arr: int keys renumber (push), string keys overwrite
```

Element read (rvalue):
```
DIM_FETCH_R   dst, arr, key       — dst = deref(arr)[norm(key)] or Null (+ undefined-key warning); no autovivify
DIM_FETCH_IS  dst, arr, key       — like _R but NO warning and Null if arr isn't an array (for `??` / nested isset)
```

Element write / nested lvalues use an **lvalue cursor stack** (per frame) so no
long-lived borrow is held into a COW `IndexMap`, and `$a[$i][$j] = v` works
without turning elements into permanent refs:

```
LVAL_CV       cv                  — push a cursor rooted at CV `cv` (autovivify Null→[] when descended)
LVAL_DIM      key                 — descend top cursor into element norm(key) (create as Null if missing; Err if scalar)?
LVAL_APPEND                       — descend top cursor into a freshly appended element
ASSIGN_LVAL   src                 — commit: write deref(src) into the cursor's slot, bubbling COW make_mut up the
                                    recorded path; pop the cursor. ref-aware at the leaf.
FETCH_LVAL_RW dst                 — read the current cursor value into dst AND keep the cursor open (for compound
                                    assign / inc-dec on `$a[$i]`): e.g. FETCH_LVAL_RW t; ADD t,t,e; ASSIGN_LVAL t
```

The cursor records *(root CV, resolved keys…)* — all key subexpressions are
evaluated into regs *before* the chain, so commit is a single `make_mut` descent
(Zend's "separation" cost) and is trivially JIT-lowerable. Reference elements
(`Value::Ref`) encountered mid-path are written through, matching PHP.

`isset`/`empty`/`unset`:
```
ISSET_CV   dst, cv                — dst = Bool(!is_null(deref(cv)))              (unset and null are indistinguishable to isset, per PHP)
ISSET_DIM  dst, arr, key          — dst = Bool(key present AND element not null); no warning, no autovivify; chains via DIM_FETCH_IS
EMPTY_CV   dst, cv                — dst = Bool(!conv::to_bool(deref(cv)))
EMPTY_DIM  dst, arr, key          — dst = Bool(!to_bool(element)) ; missing ⇒ true; no warning
UNSET_CV   cv                     — regs[cv] = Null   (NB: a never-written CV is already Null; see note)
UNSET_DIM  arr, key               — remove element via make_mut; next_int high-water NOT lowered (value-model §3c)
```

> **`unset` vs. "never defined":** `isset` treats `null` and absent identically,
> so storing `Null` on `UNSET_CV` is observably correct for `isset`/`??`/`empty`.
> The only divergence is the *undefined-variable warning* on a subsequent read.
> If byte-exact warnings on that path are required, a CV can carry an `Undef`
> sentinel; the baseline accepts `Null` and we revisit if a golden needs it.

### 4.10 foreach iteration  (phase 4) — see §9

```
FE_RESET    iter_arr, iter_pos, src, end   — snapshot deref(src) as a COW-shared array into reg iter_arr; iter_pos=Int(0);
                                             if empty, JMP end                 (by-value foreach iterates a shared snapshot)
FE_FETCH    iter_arr, iter_pos, keycv?, valcv, end
                                           — if iter_pos exhausted JMP end; else bind key (if keycv given) and value from
                                             entry at position iter_pos into the CV(s); iter_pos += 1
FE_RESET_RW cv, iter_pos, end              — by-ref foreach over the live array in CV `cv` (lvalue required)
FE_FETCH_RW cv, iter_pos, keycv?, valcv, end
                                           — promote the current element to a Ref and bind valcv to it (writes alias the element)
```

`keycv?` is an optional operand (a sentinel `Reg::MAX` means "no key target").
Position-based iteration over `IndexMap::get_index(i)` is stable for the
by-value snapshot; by-ref operates on the live CV array and leaves `Value::Ref`
elements exactly as PHP's foreach-by-ref does (value-model §4).

### 4.11 References / scope binding  (phase 6)

```
BIND_REF    dst_cv, src_cv        — `$dst =& $src`: promote src's slot to Ref(rc) (wrap current value if not already Ref),
                                    store Ref(rc.clone()) into both slots (value-model §4). dst and src now alias.
MAKE_REF    dst, cv               — materialize the CV's slot as a Ref and place Ref(rc) in dst (for `&$x` as an arg/element)
BIND_GLOBAL cv, name:NIdx         — `global $x`: bind CV to the {main} scope slot for `name`, as a shared Ref
BIND_STATIC cv, slot:u32, init:CIdx — `static $x = init`: on first execution init the persistent slot; alias CV to it after
```

### 4.12 Calls / return  (phase 5)

Calling convention: positional args are evaluated into a **contiguous window of
temp registers** `[argbase, argbase+argc)`. For a builtin, the VM passes
`&regs[argbase..argbase+argc]` straight to `NativeFn` (matching the existing
`fn(&mut dyn Host, &[Value]) -> Result<Value,Thrown>`). For a user call, that
window becomes the callee's parameter CVs, rebound per `ParamInfo`.

```
CALL_BUILTIN dst, bid, argbase, argc   — arity-check against the Builtin; r = (builtin.func)(host, &regs[argbase..+argc])?; dst = r
CALL_USER    dst, fnidx, argbase, argc — push a frame for functions[fnidx]; bind params (below); run; dst = return value
CALL_BY_NAME dst, name:NIdx, argbase, argc — resolve `name` in the function table at runtime (Err if undefined), then as CALL_USER/BUILTIN
CALL_DYN     dst, callee, argbase, argc — callee is a runtime callable (Closure / "func" string / [obj,method] array); resolve then dispatch
RET          src                        — return value = deref(src); run pending finallys (§8.3); pop frame
RET_NULL                                — return Null
```

Parameter binding in `CALL_USER` (driven by `ParamInfo`, so by-ref/variadic/named
need no per-call opcodes in the common case):

- **by value** (default): callee `CV[p] = deref(window[p])`.
- **by-ref** (`param.by_ref`): the arg must be an lvalue; the caller passed a
  `Ref` (see `SEND_REF`/`MAKE_REF` below) and the callee `CV[p]` is bound to it.
- **variadic** (`param.variadic`): collect `window[p..argc]` into a `PhpArray`
  into `CV[p]`.
- **default**: if `p >= argc`, apply `param.default` (`Const` → copy; `Init` →
  run the init block). Missing + no default ⇒ `Err(ArgumentCountError)`.

Argument-passing refinements (emitted *before* the `CALL_*` as needed):

```
SEND_REF   window_slot, cv        — place MAKE_REF(cv) into the arg window (for a by-ref parameter)
ARG_NAME   window_slot, name:NIdx — tag a window slot as a named argument; CALL_USER rebinds by ParamInfo.name
                                    (for builtins, named args are resolved to positions at compile time or rejected)
```

**Spread / dynamic argc** (`foo(...$xs)`, forwarding) can't use a static window.
When any spread is present the compiler emits the window for the fixed prefix and
then `SPREAD_ARGS src` per spread; the VM materializes a `Vec<Value>` and uses
the `*_V` call forms (`CALL_USER_V dst, fnidx, argsvec`, etc.). This dual path
keeps the overwhelmingly common fixed-arity call on the fast register-window path.

### 4.13 Exceptions  (phase 3+/5)

```
THROW        src                  — begin unwinding with deref(src) as the in-flight Throwable (enters §8 search)
THROW_MATCH_ERROR subject         — throw \UnhandledMatchError (message built from deref(subject)); for `match` fall-off
FAST_CALL    finally_pc           — push a FinallyCont onto the frame's finally stack and JMP finally_pc (enter a `finally`)
FAST_RET                          — pop a FinallyCont and dispatch it (resume after the `finally`): see §8.3
```

Catch *matching* is not an opcode: a catch landing-pad is ordinary code (the
unwinder puts the thrown value in a known reg; the pad runs `IS_INSTANCE`/class
tests — phase 7 — binds the catch variable, or re-throws). The `exception_table`
(§8) maps protected PC ranges to handler PCs; no per-`try` opcode is needed to
*enter* a try — only `THROW`, `FAST_CALL`, `FAST_RET` are explicit.

### 4.14 Misc  (phases 2, 5, 7-stub)

```
FETCH_CONST  dst, name:NIdx       — read a user/engine constant by name (Err "Undefined constant" if absent).
                                    true/false/null use LOAD_BOOL/LOAD_NULL, not this.
DECLARE_FN   fnidx                — install functions[fnidx] into the live table (conditional/non-hoisted declarations)
MAGIC_CONST  dst, which:u8        — __LINE__/__FILE__/__FUNCTION__/… resolved from the frame/proto (debug + ast-surface §4.4)
HALT                              — __halt_compiler(): stop executing this proto (as top-level `return`)
```

Object/OOP opcodes (`NEW`, `PROP_FETCH`, `METHOD_CALL`, `IS_INSTANCE`,
`CLONE`, …) are **phase 7** and intentionally omitted here; they slot into the
same encoding and calling convention.

---

## 5. Lowering patterns (how the AST becomes bytecode)

Concrete sequences; `tN` = a temporary register, `cv(x)` = the CV for `$x`.

### 5.1 Output / expressions

`echo 1 + 2;`
```
LOAD_INT  t0, 1
LOAD_INT  t1, 2
ADD       t0, t0, t1        ; ops::add
ECHO      t0
```
`$x = 5; $x += 3; echo $x;`
```
LOAD_INT  t0, 5
ASSIGN    cv(x), t0
ADD       t0, cv(x), <const/imm 3>   ; ops::add(&cv,&3)
ASSIGN    cv(x), t0                   ; ref-aware store
ECHO      cv(x)
```
`$a++;` (statement) → `POST_INC t0, cv(a)` (result discarded; `t0` is a dead temp
the allocator reuses).

Short-circuit `$a && $b` into `t0`:
```
        JMP_FALSE cv(a), Lfalse
        JMP_FALSE cv(b), Lfalse
        LOAD_BOOL t0, true
        JMP       Lend
Lfalse: LOAD_BOOL t0, false
Lend:
```
`$x ?? $y`:
```
        MOV       t0, cv(x)      ; (or DIM_FETCH_IS for array lhs — no warning)
        JMP_NOT_NULL t0, Lend
        MOV       t0, cv(y)
Lend:
```

### 5.2 Control flow

`while (C) B`:
```
Ltop:  <eval C → t0>
       JMP_FALSE t0, Lend
       <B>                       ; break→JMP Lend ; continue→JMP Ltop
       JMP Ltop
Lend:
```
`for`, `do/while` are the same shape (condition is the **last** expr of the
`for` conditions list; others evaluated for side effects). `switch` lowers to a
chain of `IS_EQ ; JMP_TRUE case_i` (loose `==`) with fall-through bodies and
`break`→`JMP Lend`. `match` lowers to `IS_IDENTICAL ; JMP_TRUE arm_i` (strict
`===`) ending in `THROW_MATCH_ERROR subject`.

`break 2;` inside two nested loops → `JMP <outer loop's Lend>` (resolved at
compile time; crosses a `finally`? see §8.3).

### 5.3 Arrays

`$a = [1, 'k' => 2, ...$b];`
```
NEW_ARRAY t0, 3
LOAD_INT  t1, 1 ; ARR_PUSH t0, t1
LOAD_INT  t1, 2 ; ARR_SET  t0, <const "k">, t1
ARR_SPREAD t0, cv(b)
ASSIGN    cv(a), t0
```
`$a[$i][$j] = $v;` (nested autovivifying store):
```
<eval $i → t0> ; <eval $j → t1>
LVAL_CV   cv(a)
LVAL_DIM  t0            ; descend/autovivify a[$i]
LVAL_DIM  t1            ; descend/autovivify a[$i][$j]
ASSIGN_LVAL cv(v)       ; commit, bubbling make_mut up the path
```
`$a[$i] += 1;`
```
<eval $i → t0>
LVAL_CV cv(a) ; LVAL_DIM t0
FETCH_LVAL_RW t1
ADD     t1, t1, <imm 1>
ASSIGN_LVAL t1
```

### 5.4 Functions (args / return / by-ref / variadic / named)

`strlen($s)` (builtin) and `foo($x, ...$rest)` (user):
```
MOV          r_base, cv(s)          ; window slot 0
CALL_BUILTIN t0, bid(strlen), r_base, 1

MOV          r_base,   cv(x)        ; positional arg 0
SPREAD_ARGS  cv(rest)               ; dynamic tail ⇒ _V path
CALL_USER_V  t0, fnidx(foo), <argsvec>
```
By-ref `sort($arr)` (param 0 by-ref): `SEND_REF r_base, cv(arr)` then
`CALL_*`. Named `foo(b: 2, a: 1)`: window filled in source order, each tagged
`ARG_NAME slot, name`, and `CALL_USER` rebinds by `ParamInfo.name`. Variadic is
entirely callee-side (`param.variadic` collects the window tail). `return $x;` →
`RET cv(x)`; `return;` → `RET_NULL`.

### 5.5 try / catch / finally / throw — see §8 for the mechanism

```
          ; exception_table: { [Ltry, Lend_try) -> Catch(Lcatch) },
          ;                   { [Ltry, Lfin)     -> Finally(Lfin) }
Ltry:     <try body>
          FAST_CALL Lfin            ; normal completion runs finally
          JMP Lafter
Lcatch:   ; thrown value placed in r_exc by the unwinder
          <type test: IS_INSTANCE … ; on no-match: THROW r_exc>
          ASSIGN cv(e), r_exc
          <catch body>
          FAST_CALL Lfin
          JMP Lafter
Lfin:     <finally body>
          FAST_RET                  ; resume: fall-through, pending jump, pending return, or rethrow
Lafter:
```

### 5.6 References

`$b =& $a;` → `BIND_REF cv(b), cv(a)`. `function f(&$p){…}` + call `f($x)` →
`SEND_REF slot, cv(x)` then `CALL_USER`; inside `f`, writes to `$p` go through
`ASSIGN cv(p), …` which write through the shared `Ref`. `foreach ($arr as &$v)`
uses `FE_RESET_RW`/`FE_FETCH_RW`. `global $g;` → `BIND_GLOBAL cv(g), name(g)`.

---

## 6. Execution model (dispatch loop sketch)

```rust
struct Frame<'p> { proto: &'p FunctionProto, regs: Vec<Value>, pc: PC, finally_stack: Vec<FinallyCont> }

fn run(&mut self, frame: &mut Frame) -> Result<Value, Thrown> {
    loop {
        let instr = frame.proto.code[frame.pc as usize];
        frame.pc += 1;                               // default: fall through
        match instr {
            Instr::Add { dst, a, b } => frame.regs[dst as usize] = ops::add(&frame.regs[a as usize], &frame.regs[b as usize])
                                            .map_err(|e| self.unwind(frame, e))??,
            Instr::Jmp { target }    => frame.pc = target,
            Instr::JmpFalse { cond, target } => if !conv::to_bool(&deref(&frame.regs[cond as usize])) { frame.pc = target; },
            Instr::Ret { src }       => { self.run_pending_finallys(frame)?; return Ok(deref(&frame.regs[src as usize])); }
            // … one arm per opcode …
        }
    }
}
```

The real loop centralizes faulting: an operator `Err` is handed to `unwind`
(§8), which either redirects `frame.pc` to a handler (and the loop continues) or
propagates the `Thrown` out of `run` (caller frame continues unwinding). `Exit`
and `Unsupported` never match a handler — they propagate straight to the SAPI.
(The `map_err(..)??` above is illustrative; the implementation threads faults
through a single `on_fault(frame, thrown) -> ControlFlow<Result<Value,Thrown>>`
helper rather than per-arm boilerplate.)

---

## 7. References, concretely

A CV slot is a `Value`. It becomes shared storage only when `&` promotes it to
`Value::Ref(Rc<RefCell<Value>>)` (value-model §4), done by `BIND_REF`/`MAKE_REF`/
`SEND_REF`/`BIND_GLOBAL`/`FE_FETCH_RW`. Thereafter:

- **read** a variable as a value → `deref` (one level), done on-use by every
  value-consuming opcode and by `ops::`/`conv::` internally.
- **write** through a variable → `ASSIGN` (and `PRE_/POST_INC/DEC`, `ASSIGN_LVAL`
  at a ref leaf): if the slot is `Ref(rc)`, write `*rc.borrow_mut()`; else
  overwrite the slot. A *new* `=&` rebinds the slot to a different `rc`.

`RefCell` borrow tracking catches the rare evaluation-order aliasing bug at
runtime instead of letting it be silent UB. Reference cycles leak under `Rc`
until the deferred cycle collector (unchanged from value-model §0).

---

## 8. Exceptions & `finally`

### 8.1 What is catchable

`Thrown` (rhp-builtins) has three arms. Only **`Thrown::Error { class, message }`**
is a PHP `Throwable` and is catchable / runs `finally`. **`Thrown::Exit(code)`**
(exit/die) and **`Thrown::Unsupported { feature, reason }`** (refused features)
are *uncatchable*: they bypass the handler search and `finally`, unwinding
straight to the SAPI (Exit → process exit code; Unsupported → the
`RHP_UNSUPPORTED` fatal). The unwinder checks the arm before searching the
table. (When phase-7 objects land, `Thrown::Error` carries the actual exception
object; catch type tests run against it. Until then they match on `class`.)

### 8.2 The exception table

```rust
pub struct ExRegion { pub start: PC, pub end: PC, pub kind: HandlerKind }  // protects [start, end)
pub enum HandlerKind { Catch { landing: PC }, Finally { landing: PC } }
```

Regions are stored **innermost-first**. On a `THROW`/operator `Err(Error(..))`,
the unwinder scans `exception_table` for the first region with
`start <= pc < end`:

- **`Catch`** — place the thrown value in the catch landing's agreed exception
  register, set `pc = landing`, resume. The landing pad does type matching and
  either binds `$e` or re-throws (which restarts the search from the enclosing
  region).
- **`Finally`** — run the finally block with a pending `Rethrow(thrown)`
  continuation (§8.3), then continue the search outward.
- **none in this frame** — pop the frame and continue unwinding in the caller
  (its `CALL_*` site is inside *its* protected ranges, so the search naturally
  resumes there). If it reaches `{main}` with no handler → uncaught fatal.

### 8.3 `finally` via FAST_CALL / FAST_RET

A `finally` block is compiled **once**. Every way of leaving the protected region
routes through it, carrying what to do *after*:

```rust
pub enum FinallyCont {
    FallThrough,          // normal completion: continue after the finally
    Jump(PC),             // a break/continue/return-less goto was crossing out
    Return(Value),        // a `return v;` was crossing out — complete it after finally
    Rethrow(Thrown),      // an exception is in flight — resume unwinding after finally
}
```

- **Normal completion** of `try`/`catch`: `FAST_CALL Lfin` pushes
  `FallThrough` (plus the real post-target), runs finally, `FAST_RET` resumes.
- **`return v;` inside `try`**: evaluate `v`, push `Return(v)`, `FAST_CALL Lfin`;
  `FAST_RET` sees `Return(v)` and performs the actual `RET` (chaining to any
  *further* enclosing finally first).
- **`break N`/`continue N` crossing a finally**: push `Jump(target)`,
  `FAST_CALL Lfin`; `FAST_RET` jumps.
- **Exception**: the unwinder pushes `Rethrow(thrown)` and enters the finally;
  `FAST_RET` resumes the outward search.

Chaining (nested finallys on one exit path) falls out naturally: each `FAST_RET`
that still needs to cross another finally issues the next `FAST_CALL`. A
`return`/`break` *originating inside* a `finally` overrides a pending
continuation (PHP semantics: `finally`'s own control flow wins), realized by the
finally body replacing the top `FinallyCont` before `FAST_RET`.

This is Zend's model and lowers cleanly to Cranelift (the finally becomes a
block with a switch on the continuation tag). `flags.has_finally` lets the common
finally-free function skip the finally stack entirely.

---

## 9. foreach iterator model

By-value `foreach` iterates a **COW snapshot**: `FE_RESET` clones the `PhpArray`
handle (an `Rc` bump) into a hidden reg, so later mutation of the original array
mid-loop does not perturb iteration (PHP's by-value foreach semantics). Position
is a hidden `Int` reg; `FE_FETCH` reads `IndexMap::get_index(pos)` and advances.
Key/value are copied into the target CV(s); destructuring targets (`as [$a,$b]`)
expand into further stores.

By-ref `foreach` (`as &$v`) operates on the **live** array in the source CV
(which must be an lvalue): `FE_FETCH_RW` promotes the current element to a
`Value::Ref` and binds `$v` to it, so writes alias the element — and the ref is
left in the array after the loop, exactly reproducing PHP's footgun
(value-model §4). Iterator state lives in frame temporaries (no new `Value`
variant), keeping `Value` unchanged and the whole thing JIT-lowerable.

---

## 10. Constants, names, and interning

- `constants: Vec<Value>` holds every literal that isn't a small inline immediate:
  string/inline-HTML bytes (as `Value::Str`), large/float literals, and constant
  parameter defaults. Deduplicated per proto.
- `names: Vec<PhpStr>` interns identifier bytes used at runtime by `NIdx`:
  variable names (for `global`/diagnostics/`compact`-style *rejected* cases),
  constant names (`FETCH_CONST`), function names (`CALL_BY_NAME`), and parameter
  names (named args). Function/const name casing follows PHP (functions
  case-insensitive; the builtin table already lowercases).
- Engine constants (`PHP_EOL`, `PHP_INT_MAX`, …) come from `builtins::constants()`
  at link time; `FETCH_CONST` consults the engine constant table, not `constants`.

---

## 11. Refused at compile time — no opcode exists

These are rejected **in the compiler**, at the AST node, with
`Thrown::Unsupported` (rendered with the `RHP_UNSUPPORTED` marker). There is
deliberately **no bytecode** that can express them, so nothing downstream (VM,
future JIT) can ever execute them:

| PHP construct | AST node | Compiler action |
|---|---|---|
| `eval(...)` | `Construct::Eval` | **refuse** — no `EVAL` opcode; executes a string as code |
| `include`/`require`(`_once`) | `Construct::{Include,IncludeOnce,Require,RequireOnce}` | refuse (deferred; filesystem/code-load) |
| backticks `` `…` `` | `CompositeString::ShellExecute` | refuse — never spawn a shell |
| `(unset)` / `(real)` / `(void)` cast | `UnaryPrefixOperator::{UnsetCast,RealCast,VoidCast}` | refuse — fatal in PHP 8.5 |
| `$$x = …` (variable-variable write) | `Variable::{Indirect,Nested}` as an assignment target | refuse the write form |

Function-level refusals (`extract`, `compact`, `create_function`, `assert`
string form, `exec`/`system`/`shell_exec`/`passthru`/`proc_open`/`popen`) have no
dedicated AST node; call-lowering checks `builtins::is_refused(name)` and emits
`Thrown::Unsupported` instead of a `CALL_*`. Everything else about a refused call
(argument evaluation order, etc.) is never compiled — the refusal is emitted in
place of the call opcode.

> This section is load-bearing per the project's explicit instruction that
> `eval` in particular is **not** to be implemented: it is absent from the
> opcode set by construction, not merely unimplemented.

---

## 12. What this ISA does **not** fix yet (deferred)

- **OOP opcodes** (`NEW`, property/const/static access, method/static calls,
  `instanceof`, `clone`, late static binding) — phase 7; they reuse the frame,
  calling convention, and exception machinery here.
- **Generators** (`yield`) — need a resumable frame (heap-allocated register
  file + saved `pc`); `flags.generator` reserves the bit. The `Frame` already
  owns its `regs`/`pc`/`finally_stack`, so suspension is "box the frame."
- **Cranelift JIT** — the stated eventual target. The `Instr` enum is the IR the
  lowering `match`es over; nothing here (typed operands, instruction-index PCs,
  side-table constants, FAST_CALL finally) blocks it.
- **Instruction packing / superinstructions / fused compare-branch** — profiling-
  driven, strictly behind this opcode set.
- **Cycle collection** — unchanged deferral from value-model §0.
```
