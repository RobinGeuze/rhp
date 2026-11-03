# Bytecode VM prior art → rhp ISA recommendations

Status: design input for the register-based bytecode compiler + VM that becomes
rhp's primary execution core. The tiny tree-walker (`rhp-runtime/src/lib.rs`)
is kept **only** as a differential oracle. Ground truth: the reference `php` (see `crates/rhp-builtins/src/version.rs`).

This document surveys four reference designs and distills concrete, rhp-specific
recommendations. It is written against the pieces we already built and will reuse
**verbatim** — do not reimplement these in the VM:

- `rhp-values`: the owned, `'static`, 16-byte `Value` enum
  (`Null/Bool/Int(i64)/Float(f64)/Str(PhpStr)/Array(PhpArray)/Object/Ref`), COW
  `PhpStr`/`PhpArray` via `Rc::make_mut`, aliasing `Ref=Rc<RefCell<Value>>` and
  objects via `Rc<RefCell>`, `ArrayKey::from_value`, `float_to_int`, `conv::*`,
  `fmt::*`.
- `rhp-builtins`: `Thrown { Error{class,message} / Unsupported{feature,reason}
  / Exit(i32) }`, the `Host` trait, `NativeFn`, `registry()`, `is_refused()`.
- `rhp-runtime/src/ops.rs`: PHP operator semantics over `&Value -> Result<Value,
  Thrown>` (`add/sub/mul/div/modulo/pow/intdiv`, bitwise, `negate`, `concat`,
  `loose_compare/loose_eq/strict_eq/spaceship`). **The VM's arithmetic/compare
  opcodes are thin wrappers over these.**

Hard constraints that shape the ISA: single-threaded (`Rc`/`RefCell`, never `Arc`);
`eval`/`extract`/`include`/backticks/`(unset)`-cast stay refused (structurally in
the compiler for construct nodes, via `is_refused()` for functions); values are
owned `'static` (the arena holds only the AST); and **the bytecode must stay
lowerable to a Cranelift JIT later** without building the JIT now.

---

## (a) Zend's opcode VM — what to borrow, what to simplify

### Operand model

Each Zend instruction (`zend_op`) is a fixed record:
`{ handler, op1: znode_op, op2: znode_op, result: znode_op, extended_value: u32,
   lineno, opcode: u8, op1_type, op2_type, result_type }`.
A `znode_op` is a **union** (a constant-table offset, a variable offset, a jump
target, a small integer, …). The three `*_type` tags select how each slot is
interpreted:

- **`IS_CONST`** — operand is an index into the op_array's literal/constant table.
- **`IS_TMP_VAR`** — a short-lived, non-addressable temporary (the result of a
  subexpression). Cannot be referenced or aliased.
- **`IS_VAR`** — a temporary that *may* hold a reference / is the result of a FETCH
  that can participate in write/ref contexts.
- **`IS_CV`** ("compiled variable") — a named local `$x`, pre-assigned a fixed slot
  in the frame's CV table at compile time. This is the key optimization: a `$x`
  access is a slot index, **not** a hash lookup of `"x"` at runtime.
- **`IS_UNUSED`** — slot absent (e.g. `$a[]` append has no key operand).

CVs, TMP_VARs and VARs all live in one per-call frame (`execute_data`); operands are
just offsets into it. Zend is **not SSA at runtime** — TMP_VAR slots are freely
reused across an op_array; SSA form (`zend_ssa`) and the control-flow graph
(`zend_cfg`) are built **only inside the opcache optimizer / JIT**, over the already-
emitted linear opcode array, from byte-offset jump targets.

`fetch_dim`/`assign` are the instructive hard part. Reads use a family of fetches
distinguished by use-context "BP type": `ZEND_FETCH_DIM_R` (read), `_W` (write/
auto-vivify), `_RW` (read-modify-write), `_IS` (isset, suppresses notices),
`_UNSET`, `_FUNC_ARG`. A write `$a[$k] = $v` compiles to `ZEND_ASSIGN_DIM`
(op1=array, op2=key) **followed by a second `ZEND_OP_DATA` opline** whose op1
carries `$v` — the value rides in a trailing pseudo-instruction because one
fixed-width opline cannot hold four operands. Compound writes chain a `FETCH_DIM_RW`
producing an `IS_VAR` place, then operate, then store back.

### The JIT

PHP 8's JIT uses **DynASM** (the LuaJIT assembler-macro tool) to emit machine code.
Two modes: a **function JIT** (whole op_array) and a **tracing JIT** (hot linear
traces across calls/loops). Type information comes from opcache's SSA-based type
inference plus, for tracing, runtime profiling. The JIT emits **type guards** (e.g.
"op1 is still a long") and, on guard failure, **side-exits** back to the generic VM
handler. Cold/complex opcodes are not open-coded — the JIT emits a **call to the
same C helper** the interpreter uses. This is the single most important lesson for
us (see section (d)).

### Borrow vs simplify

| Zend mechanism | rhp decision |
|---|---|
| **CV = named local → fixed frame slot** | **Borrow.** Resolve every `$x` to a register at compile time; never hash a variable name on the hot path. |
| **Constant/literal table per op_array, `IS_CONST` by index** | **Borrow.** Per-function const pool, deduped; folds into Cranelift constants later. |
| **Four typed operand slots on one record** | **Borrow the idea** (tagged operands) but **simplify**: our operands are a small Rust `enum`, not a C union + separate `*_type` bytes. |
| **TMP_VAR vs VAR distinction** | **Simplify to one** register file. A register can already hold a `Value::Ref` (our aliasing variant), so the "this temp might be a reference" role of `IS_VAR` is covered by the value itself, not a second operand class. |
| **5–6 `FETCH_DIM_*` BP variants** | **Simplify.** Keep at most read (`FetchDimR`), isset (`FetchDimIsset`, notice-suppressing), and *fused* write opcodes (`AssignDim`, `AssignDimOp`) that do COW + auto-vivify in one step. |
| **`ZEND_OP_DATA` trailing-opline kludge for the 4th operand** | **Drop it.** Use dedicated multi-operand opcodes (`AssignDim{arr, key, val}`), since a Rust `enum` variant can carry as many fields as the op needs. |
| **Byte-offset jump targets; CFG/SSA rebuilt later by the optimizer** | **Invert it.** Build basic blocks in the compiler *up front* (we want the JIT), branch to block ids, and treat the linear encoding as a lowering of that CFG (see (d)). |
| **Handler dispatch via `ZEND_VM_KIND` (call/goto/hybrid threading)** | **Simplify.** Rust has no guaranteed computed-goto or tail calls (`become` is unstable); use a dense `match op { .. }` loop — LLVM lowers a dense discriminant match to a jump table. Revisit threading only if profiling demands it. |

---

## (b) Register VMs vs stack VMs — why register suits PHP

**Lua 5.x** is the canonical register VM. Fixed **32-bit** instructions, fields
`OpCode(6) | A(8) | B(9) | C(9)` (or `Bx(18)`/`sBx` for jumps/constants). Up to
**256 registers per function** (8-bit `A`), which *are* the function's stack
window: locals occupy low registers, expression temporaries are allocated at the
top of the window (`freereg` bump/pop in `lcode.c`). The `B`/`C` operands use the
**RK trick**: the high bit selects register-vs-constant, so `a = b + 1` is one
`ADD A B K` without a separate load. Constant folding happens in the code generator
(`luaK_*`). The opcode set is small (~40). The payoff vs a stack VM is that
`local x = a + b` is **one** `ADD` instruction naming its three operands, instead of
`GETLOCAL a; GETLOCAL b; ADD; SETLOCAL x` — fewer instructions, fewer dispatches, and
operands are **explicit** rather than implied by stack position.

Other register designs worth citing: **Dalvik/ART** (register Java bytecode, up to
64K `vN` registers, chosen precisely because register form AOT/JIT-compiles well —
the opposite choice from stack-based JVM bytecode) and **V8 Ignition** (register
bytecode feeding the TurboFan JIT). Both picked register form *for compilability*.

**Correction on the brief:** **Wren** is actually a *single-pass **stack** VM*
(Bob Nystrom), not a register VM. Its lesson for us is compiler *simplicity* (one
pass, no separate IR), not register allocation — and it is a useful warning: a
single-pass stack compiler is lovely for a scripting language but gives the JIT
nothing to chew on. **CPython** is the stack-VM reference proper: everything flows
through a value stack; 3.11's specializing adaptive interpreter (PEP 659) is relevant
for section (c), not as a register model.

**Why register suits PHP specifically:**

1. **PHP expressions are temporary-dense.** Zend already models this with TMP_VAR.
   A register file is the natural home for those temporaries; a stack would spend
   most dispatches on push/pop churn that a JIT then has to undo.
2. **Our operators already have register shape.** `ops::add(&Value, &Value) ->
   Result<Value, Thrown>` *is* a 3-address operation: `frame[dst] = ops::add(
   &frame[a], &frame[b])?`. A stack VM would force an awkward pop-pop-push wrapper
   around the exact same call. Register form is a 1:1 fit with code we already have.
3. **Compound lvalue work (fetch → op → store) is 3-address by nature.** Reading
   `$a[$k]` into a temp, applying `ops::*`, and storing back maps cleanly to
   `FetchDimR r_t, r_a, r_k` / `BinOp r_t, r_t, r_v` / `AssignDim r_a, r_k, r_t`.
4. **Explicit operands = a head start for Cranelift.** Register indices become SSA
   values or stack slots directly; stack-relative operands would first have to be
   reconstructed into named values.

Recommendation: **register-based, 3-address, one unified frame register file**
(CVs = named locals in low registers; temporaries above them). Keep the opcode set
small and Lua-sized. **Do not** adopt Lua's RK high-bit packing: prefer an explicit
tagged `Operand` for readability and because a uniform operand layout matters more to
the JIT than instruction density right now (we can pack to `u32` fields later without
semantic change).

---

## (c) Inline caches — leaving room in the ISA

PHP resolution sites that are expensive and benefit from caching: **property
offset** (`$o->p` → slot index in the object's property table, by name), **method
resolution** (`$o->m()` → `(ClassId, name) → FnDef`), **function resolution**
(`foo()` → `FnDef`), **class-constant** and **static-property** lookup. Each is a
name→target map keyed on a runtime class/scope that is *usually stable* at a given
site (monomorphic).

Two reference mechanisms:

- **Zend run-time cache.** Each op_array has a `run_time_cache` region; opcodes like
  `ZEND_FETCH_OBJ_*` and `ZEND_INIT_METHOD_CALL` reserve **cache slots** (an offset
  stored in the opline). On first execution the slot is filled with `(cached_class,
  resolved_offset_or_fn)`; subsequent hits compare the object's class pointer against
  `cached_class` and, on a match, use the cached offset with no lookup. It is a
  *side* cache (indexed from the instruction), not stored in the instruction stream.
- **CPython 3.11 (PEP 659).** Caches are **interleaved into the bytecode array**
  immediately after a specialized instruction, with an adaptive counter driving
  (de)specialization. This keeps caches hot in icache but makes the instruction
  stream mutable and non-uniform.

**Recommendation for rhp:** take Zend's **side-cache** shape, not CPython's
inline interleaving — it keeps instructions **fixed-shape and immutable/shareable**
(important for the JIT and for not rewriting code we execute).

- Give the relevant opcodes a `cache: u32` field that indexes a per-compiled-function
  `Vec<InlineCache>` side table. Reserve the field **now**, even in phases before
  objects exist, for `Call` (function resolution) so the ISA doesn't change shape
  later; property/method cache sites are added with phase 7.
- Single-threaded ⇒ **no atomics**. A cache entry is plain interior mutability:
  ```rust
  struct PropCache  { class: Cell<Option<ClassId>>, slot: Cell<u32> }
  struct CallCache  { class: Cell<Option<ClassId>>, func: Cell<Option<FnRef>> }
  ```
- The guard is a `ClassId` (`u32`) compare, **not** a pointer compare — our object
  model already keys classes by `ClassId`, so guards are trivially cheap and stable
  across GC/`Rc` churn. This is a concrete advantage over Zend's pointer guards.
- Start **monomorphic** (one `(class → target)` pair with a guard + slow-path
  fallback). The field is wide enough to later point at a small polymorphic cache
  bucket without an ISA change.
- The slow path on a miss is a call into the ordinary resolver (the same code the
  tree-walker oracle uses), which also fills the cache — mirroring Zend.

---

## (d) JIT-amenability — keeping the Cranelift door open (without building it)

Cranelift consumes **SSA with basic blocks and block parameters**; it cannot see
Rust's `?`-based `Thrown` unwinding, implicit fall-through, or a tagged-enum match as
control flow. The bytecode is cheap to lower **iff** it already exposes that
structure. Concrete ISA properties to adopt now:

1. **Uniform, fixed-shape operands.** Every opcode's operands are the same small set
   of kinds (`Reg(u32)`, `Const(u32)`, `Block(u32)`, `CacheIdx(u32)`, small
   immediates). A JIT pattern-matches one opcode → one clif template. Register
   indices lower to stack slots (pre-regalloc) or SSA values (post). Avoid
   context-dependent operand meanings (Zend's BP-typed fetches) and avoid the
   `OP_DATA` trailing-operand trick — both force the lowerer to look across
   instructions.

2. **Explicit basic blocks, explicit terminators, no implicit fall-through.** Compile
   to a CFG of basic blocks; **every block ends in exactly one terminator**
   (`Jump`, `Branch{cond, then, else}`, `Return`, `Throw`, or a call that can throw —
   see (4)). No block is entered in the middle; branch targets are **block ids**, not
   raw byte offsets. (Zend recovers this in the optimizer; we produce it directly.)
   This is a near-1:1 match for Cranelift blocks and removes the "fall-through
   surprise" class of bugs entirely.

3. **Explicit types and guards as first-class opcodes.** The `Value` enum is tagged;
   good native code needs the tag resolved. Even while the VM runs the generic
   `ops::*` helpers, reserve **type-specialized opcode variants** (`AddInt`,
   `AddFloat`, `LtInt`, …) and/or a **type-feedback side channel** (per-site observed
   types, like the inline caches in (c)). The JIT then emits a guard (`is Int`) +
   open-coded fast path + side-exit to the generic opcode. Design the opcode set so a
   specialized op is a drop-in for its generic sibling (same operands), so
   specialization never reshapes the stream.

4. **Model exceptions as explicit edges, not Rust `?`.** `Thrown` propagation via
   `Result`/`?` is invisible to a JIT. Carry a per-function **exception region
   table** — `{ start_block, end_block, handler_block, catch_type }` — built from
   `try/catch` (the JVM exception table / Zend's live-range approach). The
   interpreter consults it to route an `Err(Thrown)` to the right handler block; the
   JIT uses it to emit real CFG edges from any throwing op to its landing pad.
   Opcodes that can throw (every `ops::*` wrapper, `AssignDim`, calls, `Throw`) are
   thus **block terminators with an exception successor** when inside a protected
   region. `finally` is a block reached on both normal and exceptional paths.

5. **Clean runtime-helper ABI for every opcode.** This is Zend's decisive trick:
   the JIT open-codes only the hot integer/float/guarded paths and **calls the exact
   same Rust function** for everything else. So keep each opcode's semantics behind a
   plain, call-shaped Rust function over `&Value`/`&mut Frame` — which we already have
   in `ops::*` and the builtin `NativeFn`. Never bury semantics in the interpreter
   loop in a way a helper can't be extracted for. COW/`Rc` refcount traffic stays
   inside those helpers; the JIT initially lowers an opcode to a helper call and only
   later inlines the fast path, exactly as Zend does.

6. **Fixed per-function frame layout + uniform calling convention.** A function
   declares its register count up front (CVs + max temps); the frame is a
   `Vec<Value>` of that size. Calls pass arguments in a **contiguous register range**
   so a JIT can set up callee frames uniformly (Lua/Zend both do this). Const pool by
   index → clif constants.

7. **Refusals are a compile-time *absence of opcode*, not a runtime branch.** `eval`,
   backticks, `(unset)`/`(void)`/`(real)` casts, and `$$x = …` writes are refused
   **structurally in the lowerer**: no opcode is ever emitted for them (the compiler
   raises `Thrown::Unsupported` at lowering time); `extract`/`compact`/
   `create_function`/shell funcs are refused via `is_refused()` at call lowering.
   Because the dangerous constructs never enter the ISA, the JIT never has to reason
   about them — the door we keep *shut* stays shut by construction.

---

## Distilled ISA recommendation

- **Shape:** register-based, 3-address, one per-call frame register file
  (`Frame = Vec<Value>`); CVs = named locals in low slots (name→reg resolved at
  compile time), temporaries above. Small Lua-sized opcode set.
- **Instruction:** a Rust `enum Op` whose variants carry explicit tagged operands
  (`Reg(u32)`, `Const(u32)`, `Block(u32)`, `CacheIdx(u32)`, small immediates).
  Readability + uniform layout first; pack to fixed-width `u32` fields later if the
  interpreter loop needs it, with no semantic change. No RK high-bit packing, no
  `OP_DATA` trailing-operand kludge.
- **Arithmetic/compare/concat opcodes are thin wrappers over `ops::*`**:
  `frame[dst] = ops::add(&frame[a], &frame[b])?`. `Thrown` rides `Err`, as today.
  Reserve type-specialized siblings (`AddInt`/`AddFloat`/…) for the JIT.
- **Lvalues:** fused write opcodes (`AssignDim{arr,key,val}`, `AssignDimOp`,
  `AppendDim`) that do `PhpArray::make_mut` COW + auto-vivify in one op; at most
  `FetchDimR` / `FetchDimIsset` on the read side. References reuse `Value::Ref`
  (`Rc<RefCell<Value>>`); a `MakeRef` opcode promotes a slot. All single-threaded.
- **Control flow:** CFG of basic blocks, block-id targets, one explicit terminator
  per block, no implicit fall-through.
- **Exceptions:** per-function region table `{start, end, handler, catch_type}`;
  throwing ops inside a region carry an exception successor edge.
- **Inline caches:** Zend-style **side** table (`Vec<InlineCache>`) indexed by a
  `cache: u32` opcode field; `ClassId` guards, `Cell`-based, monomorphic to start.
  Reserve the field on `Call` from the first function phase.
- **Const pool** per function, deduped, by index.
- **Dispatch:** a dense `match op` loop (Rust lacks guaranteed computed-goto/tail
  calls); revisit only if profiling warrants.
- **Refusals** stay structural in the lowerer (construct nodes) + `is_refused()`
  (functions); refused constructs never acquire an opcode.
- **Oracle:** the existing tree-walker stays as the differential test oracle against
  `php` 8.5.8; every VM opcode's result is validated byte-for-byte through it.

Net: borrow Zend's CV/const-pool/side-cache/helper-ABI, borrow Lua's small 3-address
register core, reject Zend's operand-type explosion and `OP_DATA` kludge and CPython's
inline-interleaved mutable caches, and front-load the basic-block CFG + exception
table + helper-shaped opcodes so a later Cranelift backend is a lowering job, not a
redesign.
