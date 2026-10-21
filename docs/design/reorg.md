# phrust — crate reorganization & migration plan (tree-walker → bytecode VM)

Status: design. Supersedes the 5-crate layout in `README.md §1` once executed.

This plan reorganizes the workspace so the **register VM** is the primary
execution core and the tree-walker becomes a **differential oracle**, with the
tier-agnostic pieces (`Value`, `conv`, `fmt`, operator semantics, builtins)
reused *verbatim* by both — no duplication, no cycles. The bytecode stays a flat
data ISA so a Cranelift JIT can lower it later.

> **Refusal invariant (unchanged, reaffirmed).** `eval()`, `include`/`require`,
> backticks (shell-exec), and the `(unset)` cast are **not implemented and never
> will be on this path.** The VM path refuses them *structurally in the
> compiler* (the compiler matches the `Construct::Eval` / `Construct::Include*` /
> backtick / `UnsetCast` AST nodes and emits `Thrown::Unsupported` instead of
> lowering them); the oracle keeps its existing node-level refusals. Dynamic
> functions (`extract`/`compact`/`create_function`/`assert`/shell funcs) stay
> refused via `builtins::is_refused()`. The `eval_*` → `walk_*` rename below
> exists precisely so that "walk the AST" is never confused with PHP `eval()`,
> which we refuse.

---

## 1. Current layout (as built)

```
phrust-cli  (bin; main.rs is an empty stub)
   └─> phrust-runtime, phrust-frontend, phrust-values
phrust-runtime   src/lib.rs (tree-walker, Interp, Flow) + src/ops.rs (operators)
   └─> phrust-values, phrust-frontend, phrust-builtins, mago-syntax
phrust-builtins  src/lib.rs (Thrown, Host, NativeFn, registry, is_refused) + src/dump.rs
   └─> phrust-values
phrust-frontend  src/lib.rs (ParsedFile + arena)   └─> mago-*
phrust-values    src/{lib.rs (Value/PhpStr/PhpArray/ArrayKey), conv.rs, fmt.rs}
   └─> bstr, indexmap, rustc-hash
```

Key facts that drive the reorg:

- `phrust-runtime/src/ops.rs` is tier-agnostic operator semantics
  (`add/sub/.../loose_compare/strict_eq/spaceship`). It returns
  `Result<Value, Thrown>`, so it **needs `Thrown`**, which today lives in
  `phrust-builtins`. Both the VM and the oracle must call these functions.
- `phrust-runtime/src/lib.rs` is the tree-walker. It is the *only* thing that is
  genuinely tier-specific (it is the oracle). Its `eval_*` methods and `ops`
  submodule are tangled together in one crate.
- `conv` and `fmt` already live in `phrust-values` (pure, no `Thrown`). They do
  **not** need to move.
- The 21 goldens are `.php`/`.out` pairs under `tests/golden/`. There is **no
  Rust runner yet** (`phrust-cli/src/main.rs` is `fn main() {}`). "Goldens
  passing under the VM" therefore means *building* the differential runner.

---

## 2. The `Thrown`-location decision (flagged explicitly)

**Decision: move `Thrown` DOWN into `phrust-values`** (new module
`phrust_values::thrown`, re-exported as `phrust_values::Thrown`).

Reasoning, including the exact question asked:

- *Does moving `ops` into `phrust-values` force `Thrown` into `phrust-values`
  too?* **Yes** — `ops` returns `Result<Value, Thrown>` for
  `DivisionByZeroError`/`TypeError`/overflow, so wherever `ops` lands, `Thrown`
  must be *at or below* it. If `ops` went into `phrust-values`, `Thrown` would
  have to follow.
- We are **not** putting `ops` in `phrust-values` (see §3 — it goes in a new
  `phrust-semantics` crate), but we move `Thrown` to `phrust-values` **anyway**,
  for two independent reasons:
  1. **It breaks the latent cycle.** `ops` must be reusable by *builtins* too
     (the current `cmp_loose` shortcut in `fn_max/fn_min` is a known-incomplete
     stand-in for `ops::loose_compare`; correct `max/min/sort` need the real
     thing). If `Thrown` stayed in `phrust-builtins`, then `phrust-semantics`
     (holding `ops`) would depend on `phrust-builtins` for `Thrown`, while
     `phrust-builtins` would want `phrust-semantics` for `ops` → **cycle**.
     Putting `Thrown` in `phrust-values` (below both) makes
     `semantics → values` and `builtins → values, semantics` cleanly acyclic.
  2. **Correct layering.** `Thrown` is the engine's fundamental unwind type
     (real throwables + `Unsupported` + `Exit`). The compiler, bytecode decoder,
     and VM all need to *name* it but have no business depending on the whole
     `phrust-builtins` crate. It belongs with the value model.

`Host`, `NativeFn`, `Builtin`, `registry()`, `constants()`, `is_refused()`,
and `dump` **stay in `phrust-builtins`** — they are the native-function layer,
not the value model. `phrust-builtins` keeps `pub use phrust_values::Thrown;`
so existing `builtins::Thrown` paths keep compiling during and after migration.

---

## 3. Target workspace

Nine crates. `phrust-runtime` is **dissolved**: `ops.rs` → `phrust-semantics`,
the tree-walker → `phrust-oracle`.

| crate | role | depends on |
|---|---|---|
| `phrust-values` | `Value`, `PhpStr`, `PhpArray`, `ArrayKey`, `conv`, `fmt`, **`Thrown`** | `bstr`, `indexmap`, `rustc-hash` |
| `phrust-frontend` | `ParsedFile` + arena (mago wrapper) | `mago-*` |
| `phrust-semantics` | operator semantics (`ops`) + shared value helpers | `phrust-values` |
| `phrust-builtins` | `Host`, `NativeFn`, `registry`, `constants`, `is_refused`, `dump` | `phrust-values`, `phrust-semantics` |
| `phrust-bytecode` | ISA: opcodes, register/const-pool types, chunk format (data only) | `phrust-values` |
| `phrust-compiler` | AST → bytecode; **owns structural refusal** of eval/include/backtick/(unset) | `phrust-frontend`, `phrust-bytecode`, `phrust-values` |
| `phrust-vm` | the register VM (executes chunks, implements `Host`) | `phrust-bytecode`, `phrust-semantics`, `phrust-builtins`, `phrust-values` |
| `phrust-oracle` | the tree-walker (ex-`phrust-runtime`), `walk_*` methods | `phrust-frontend`, `phrust-semantics`, `phrust-builtins`, `phrust-values` |
| `phrust-cli` | SAPI + differential runner (php ⟷ vm ⟷ oracle) | `phrust-frontend`, `phrust-compiler`, `phrust-bytecode`, `phrust-vm`, `phrust-oracle`, `phrust-builtins`, `phrust-semantics`, `phrust-values` |

### Dependency graph (acyclic)

```
                         phrust-cli
        ┌──────────┬──────────┼──────────┬───────────┐
        v          v          v          v           v
  phrust-compiler phrust-vm  phrust-oracle        (frontend, values…)
     │   │  │       │  │ │        │  │  │
     │   │  │       │  │ └────> phrust-builtins <───┘
     │   │  │       │  └──────> phrust-semantics <──┐
     │   │  └───────┴─────────> phrust-bytecode     │
     │   └── phrust-frontend        │               │
     └──────────┬───────────────────┴───────────────┘
                v
          phrust-values   (leaf: holds Thrown, conv, fmt, Value)
```

Edge rules that keep it acyclic and JIT-friendly:

- **`phrust-values` is the single leaf** everything funnels to; it gains
  `Thrown` and nothing else.
- **`phrust-bytecode` depends only on `phrust-values`** (its const pool holds
  owned `Value`s; opcodes/registers are plain data). It has **no behavior**, no
  `Host`, no `frontend` — so a future `phrust-jit` can depend on it + `values` +
  `semantics` to lower the same ISA without pulling in the compiler or VM.
- **`phrust-compiler` does not depend on `phrust-vm`/`phrust-oracle`/
  `phrust-builtins`.** It needs `Thrown` (from `values`) to emit refusals and
  `is_refused()`? No — function-name refusal is a *runtime* concern (names may be
  dynamic); the compiler only refuses the *syntactic constructs* (eval/include/
  backtick/unset-cast nodes). So compiler stays off `builtins`.
- **`phrust-vm` does not depend on `phrust-frontend`** — it runs bytecode, never
  the arena. **`phrust-oracle` does not depend on `phrust-bytecode`/`compiler`/
  `vm`** — the two tiers never see each other; only the CLI knits them.
- `phrust-builtins → phrust-semantics` is the edge enabled by moving `Thrown`
  down; it lets `max/min` (and future `sort`) call `ops::loose_compare` instead
  of the incomplete local `cmp_loose`.

### What `phrust-semantics` holds

- `ops.rs` verbatim (arithmetic/comparison/bitwise/concat, `intdiv`), its `use
  phrust_builtins::Thrown` rewritten to `use phrust_values::Thrown`.
- The **shared value helpers** currently duplicated as free fns in the
  tree-walker, lifted so the VM reuses them (not reimplements them):
  `deref`, `inc_dec_value`, `cast_to_array`, and `ArrayKey::from_value`-based
  offset coercion. These are tier-agnostic and both VM + oracle need them.

---

## 4. What moves where (file-level)

| from | to | notes |
|---|---|---|
| `phrust-builtins/src/lib.rs` → `Thrown` enum + `impl Thrown` | `phrust-values/src/thrown.rs` | add `pub mod thrown; pub use thrown::Thrown;` to values `lib.rs`; builtins keeps `pub use phrust_values::Thrown;` |
| `phrust-runtime/src/ops.rs` | `phrust-semantics/src/ops.rs` (+ `lib.rs` re-export) | rewrite `use phrust_builtins::Thrown` → `use phrust_values::Thrown` |
| `phrust-runtime/src/lib.rs` (tree-walker) | `phrust-oracle/src/lib.rs` | `mod ops;` → `use phrust_semantics::ops;`; apply §5 renames |
| `phrust-runtime` (crate) | **deleted** | removed from `members`; its two files relocated above |
| (new) | `phrust-bytecode/src/{lib.rs,isa.rs,chunk.rs}` | ISA types; no logic |
| (new) | `phrust-compiler/src/{lib.rs,lower.rs,refuse.rs}` | `refuse.rs` holds the structural eval/include/backtick/unset refusal |
| (new) | `phrust-vm/src/{lib.rs,exec.rs,frame.rs}` | register machine; `impl Host for Vm` |
| `phrust-cli/src/main.rs` (stub) | `phrust-cli/src/{main.rs,runner.rs}` | SAPI + differential golden runner |
| `phrust-values`, `phrust-frontend`, `phrust-builtins/dump.rs` | unchanged in place | kept verbatim |

---

## 5. Exact rename list (oracle, `phrust-oracle/src/lib.rs`)

Mandated `eval_* → walk_*` (methods on `Interp`, plus the free `eval_literal`):

| current | new |
|---|---|
| `fn eval_expr` | `fn walk_expr` |
| `fn eval_binary` | `fn walk_binary` |
| `fn eval_unary_prefix` | `fn walk_unary_prefix` |
| `fn eval_unary_postfix` | `fn walk_unary_postfix` |
| `fn eval_conditional` | `fn walk_conditional` |
| `fn eval_array` | `fn walk_array` |
| `fn eval_constant` | `fn walk_constant` |
| `fn eval_construct` | `fn walk_construct` |
| `fn eval_call` | `fn walk_call` |
| `fn eval_assignment` | `fn walk_assignment` |
| `fn eval_literal` (free fn) | `fn walk_literal` |
| `type EvalResult` | `type WalkResult` |

Not renamed / explicitly preserved:

- `exec_stmt`, `exit_code_from`, `pre_inc_dec`, `post_inc_dec`, `read_variable`,
  `run`, `output`, `Flow`, `Interp` — keep names. (`exec_stmt` may optionally
  become `walk_stmt` for symmetry, but it is not an `eval_*` name; left as-is to
  minimize churn.)
- **`Construct::Eval(_)` match arm stays** and keeps returning
  `Thrown::unsupported("eval()", …)`. The rename is *about the walker methods*,
  not about un-refusing PHP `eval()` — that construct remains refused. (In the
  VM path the same refusal is emitted by `phrust-compiler::refuse`.)
- `ops::*` function names are unchanged; only their crate path moves
  (`phrust_runtime::ops` → `phrust_semantics::ops`).

---

## 6. Migration checklist (cargo green at every step)

Each step ends with `cargo build --workspace && cargo test --workspace` passing.
Steps 1–4 are pure refactors of existing code (goldens unaffected because the
runner doesn't exist yet); steps 5–9 build the VM path and wire the runner.

**Step 0 — baseline.** `cargo build --workspace` green (confirmed; one dead-code
warning on `ops::intdiv`). Snapshot current behavior: add a throwaway test (or
reuse `phrust-runtime` unit tests) proving the tree-walker output for a couple
of goldens, so later steps have a regression anchor.

**Step 1 — move `Thrown` to `phrust-values`.**
Create `phrust-values/src/thrown.rs` with the `Thrown` enum + impls; add
`pub mod thrown; pub use thrown::Thrown;` to values `lib.rs`. In
`phrust-builtins/src/lib.rs` delete the enum, add `pub use
phrust_values::Thrown;`. No other crate changes (everyone still says
`builtins::Thrown`). Build green.

**Step 2 — create `phrust-semantics`, move `ops`.**
New crate `phrust-semantics` (dep: `phrust-values`); move
`phrust-runtime/src/ops.rs` into it; change its `use phrust_builtins::Thrown` to
`use phrust_values::Thrown`; re-export from `lib.rs`. In `phrust-runtime`,
replace `mod ops;` with `use phrust_semantics::ops;` and add the dep. Build
green (tree-walker now calls `ops` from the shared crate). Silence/364 the
`intdiv` dead-code warning by exposing it (the VM will use it).

**Step 3 — lift shared value helpers into `phrust-semantics`.**
Move `deref`, `inc_dec_value`, `cast_to_array` out of the tree-walker into
`phrust-semantics` (as `pub fn`s); update the tree-walker to call them. Build
green. (This is what prevents the VM from re-implementing them in step 7.)

**Step 4 — rename crate `phrust-runtime` → `phrust-oracle`; apply `walk_*`.**
Rename the directory + `Cargo.toml` `name`; update `phrust-cli` dep
`phrust-runtime` → `phrust-oracle`. Apply the §5 renames (mechanical; the
`Construct::Eval` refusal arm stays). Update `phrust-builtins → phrust-semantics`
dep so `fn_max/fn_min` can later use `ops::loose_compare` (optional to rewrite
now; the dep edge is what matters). Build + test green — the oracle is now a
standalone, correctly-named crate reusing `values`/`semantics`/`builtins`.

**Step 5 — add `phrust-bytecode` (ISA only).**
New crate (dep: `phrust-values`). Define the register ISA: `Reg(u32)`,
`ConstIdx`, `Chunk { consts: Vec<Value>, code: Vec<Op>, num_regs, .. }`, and the
`Op` enum covering what the 21 goldens exercise — `LoadConst`, `Move`,
arithmetic/comparison/bitwise/concat ops (one variant per `ops::` fn), `Not`,
`Neg`, `BitNot`, `Concat`, `Jump`/`JumpIfFalse` (for `?:`/`??`/`&&`/`||`
short-circuit), `CallBuiltin{name_idx, argc}`, `Echo`, `Print`, `IncDec`,
`CastInt/Float/String/Bool/Array`, `Return`, `Exit`. Data only, no methods that
execute. Build green (nothing depends on it yet).

**Step 6 — add `phrust-compiler` (+ structural refusal).**
New crate (deps: `phrust-frontend`, `phrust-bytecode`, `phrust-values`).
`lower.rs` walks the AST and emits a `Chunk`, mirroring the oracle's expression
coverage (literals, binary with short-circuit, unary, casts, variables as
register slots, assignment/compound-assignment, conditional/elvis/coalesce,
array literals, constant access, `echo`/`print`/`exit`, builtin calls). `refuse.rs`
pattern-matches and returns `Err(Thrown::unsupported(..))` for `Construct::Eval`,
`Construct::Include*`/`Require*`, backtick/shell-exec expressions, and the
`(unset)` cast node — **the VM path's structural refusal lives here.** Add a unit
test that each refused construct yields `Unsupported`. Build + test green.

**Step 7 — add `phrust-vm`.**
New crate (deps: `phrust-bytecode`, `phrust-semantics`, `phrust-builtins`,
`phrust-values`). Register file = `Vec<Value>`; a `vars: HashMap<Vec<u8>,Value>`
scope as in the oracle; `impl Host for Vm { fn write_out }`. The decode loop
dispatches each `Op` to the **shared** `phrust_semantics::ops::*` (never
reimplements arithmetic) and to `phrust_builtins::lookup`/arity-check/`is_refused`
for calls (identical rules to the oracle). `Exit`/throws ride `Err(Thrown)` to
the SAPI exactly as today. Unit-test the VM against the oracle on a handful of
expressions. Build + test green.

**Step 8 — wire `phrust-cli` differential runner.**
`phrust-cli` depends on compiler + bytecode + vm + oracle (+ values/builtins/
semantics/frontend). `main.rs`: a real SAPI — read a `.php` file, parse, compile,
run on the VM, write stdout, exit with the code. `runner.rs`: for each
`tests/golden/*.php`, run (a) the local `php` 8.5.8 binary, (b) the VM, (c) the
oracle, and assert all three byte-identical to the `.out` (and to each other).
Add it as `#[test]` (or an xtask) so `cargo test` drives the goldens. Build
green; run the suite.

**Step 9 — green the 21 goldens under the VM.**
Iterate on `phrust-compiler` + `phrust-vm` until all 21 match `php`/`.out`/oracle.
The goldens are scalar/operator/dump heavy (arith, casts, comparison, spaceship,
`var_dump`/`print_r`/`var_export`, `is_numeric`, overflow) — all covered by the
shared `ops`, `conv`, `fmt`, and `dump` the VM reuses, so divergences should be
compiler-lowering bugs, not semantic ones. Delete `phrust-runtime` from
`members` (already renamed in step 4; confirm no stale references). Final
`cargo build --workspace && cargo test --workspace` green with VM == oracle == php
on all 21.

---

## 7. Why this is JIT-ready (not foreclosed)

`phrust-bytecode` is pure data over `phrust-values`, with `ops`-aligned opcodes
and a flat const pool. A later `phrust-jit` crate depends on
`phrust-bytecode` + `phrust-semantics` + `phrust-values` and lowers the *same*
`Chunk` to Cranelift IR, calling the identical `phrust_semantics::ops::*` as
runtime helpers for the slow paths — no change to the compiler, the ISA, or the
value model is required. The VM and the JIT remain two consumers of one ISA, the
way the VM and oracle are two consumers of one `ops`.
