# phrust — VM execution architecture

The execution core is a **register-based bytecode compiler + VM**. The
tree-walker in `phrust-runtime/src/lib.rs` is demoted to a *differential oracle*
(`Interp`): it stays byte-for-byte correct on the subset it covers and is run
side-by-side with the VM in tests, but it is no longer the product.

This document specifies the VM. It is a contract on top of the four existing
design docs and the already-built, **tier-agnostic** crates the VM reuses
verbatim:

- `phrust-values` — the owned `Value` enum, `PhpStr`/`PhpArray`/`PhpRef`,
  `ArrayKey` + key normalization, `conv::`, `fmt::`. The VM stores these in
  registers; it never invents a second value representation.
- `phrust-builtins` — `Thrown`, `Host`, `NativeFn`, `registry()`, `constants()`,
  `is_refused()`. The VM **is** the `Host`; it dispatches builtins through the
  registry.
- `phrust-runtime::ops` — every binary/unary PHP operator over `Value`. Each
  arithmetic/compare/concat/bitwise opcode is a thin wrapper that calls the
  matching `ops::` function. **The VM must not reimplement operator semantics.**

Non-negotiable invariants carried over unchanged: single-threaded (`Rc`/`RefCell`,
never `Arc`); `Value`s are owned and `'static`; the arena holds only the AST;
`eval()` / `extract` / `compact` / `create_function` / `include`/`require` /
backticks / `(unset)`-cast stay **refused** — structurally in the compiler for
the construct nodes, via `is_refused()` for the function names. `eval` is
*never* implemented; the compiler lowers `Construct::Eval` straight to a
`Thrown::Unsupported`, exactly as the oracle does today.

The bytecode is designed to be lowerable to a **Cranelift JIT later**: opcodes
are typed, 3-address, SSA-friendly over a flat register file, and operand types
are discoverable. We do not build the JIT now, but no decision here forecloses
it (see §7).

---

## 0. Crate placement

The VM lives in `phrust-runtime` alongside the oracle (same dependency edges —
it already sees `phrust-values`, `phrust-frontend`, `phrust-builtins`). New
modules:

```
crates/phrust-runtime/src/
  lib.rs        // re-exports; the oracle Interp stays here (becomes `oracle`)
  ops.rs        // UNCHANGED — reused by both tiers
  vm/
    mod.rs      // Vm struct, run loop, Frame, call stack
    instr.rs    // Instr enum, Proto, Handler, exception table
    compile.rs  // AST (mago cst) -> Proto / Program ; refusals live here
    program.rs  // compiled Program module (main + functions + classes)
  engine.rs     // the shared differential entrypoint (oracle + vm)
```

No new crate, no new dependency. `phrust-cli` picks the tier behind a flag
(`--engine=vm|oracle`, default `vm`).

---

## 1. The interpreter loop

### 1.1 Dispatch style

**Now:** a `match` on an owned `Instr` enum inside one `loop`. This is the
portable baseline, it keeps the opcode set readable, and `rustc` already lowers
a dense `match` to a jump table. No `unsafe`, no macro soup.

**Later (noted, not built):** the hot loop is structured so it can be swapped for
(a) a computed-goto / token-threaded dispatch (requires either a stable label-as-
value or an opcode-indexed `[fn; N]` table — feasible once the opcode set
freezes), or (b) per-opcode tail-called handler functions
(`fn(op_handler) -> &next`), which LLVM turns into threaded dispatch and which
also mirrors how the Cranelift tier will shape basic blocks. The `match` arms are
each a self-contained "read operands → compute → write dst → advance pc", so any
of these lowerings is mechanical.

### 1.2 The VM struct

```rust
use std::collections::HashMap;
use std::rc::Rc;

use phrust_builtins::{self as builtins, Host, Thrown};
use phrust_values::{ClassId, Value};

pub struct Vm {
    /// The single output sink. `impl Host` appends here; the SAPI flushes it.
    out: Vec<u8>,

    /// The explicit PHP call stack (see §2). NOT native recursion.
    stack: Vec<Frame>,

    /// User-defined functions, keyed by lowercase name (PHP fns are
    /// case-insensitive). Compiled once, shared via `Rc<Proto>`.
    functions: HashMap<Vec<u8>, Rc<Proto>>,

    /// Engine + user constants (case-sensitive; keyword literals handled in the
    /// compiler, not here). Seeded from `builtins::constants()`.
    constants: HashMap<Vec<u8>, Value>,

    /// Class table (phase 7). `ClassId` indexes `classes`; names resolve through
    /// `class_ids`. Objects store a `ClassId`, never a class pointer.
    classes: Vec<ClassEntry>,
    class_ids: HashMap<Vec<u8>, ClassId>,

    /// Recursion guard -> PHP `Error: Maximum call stack size` instead of a
    /// Rust stack overflow (the explicit stack makes this a cheap length check).
    max_depth: usize,
}

impl Host for Vm {
    fn write_out(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }
}

impl Vm {
    pub fn new() -> Self {
        let mut constants = HashMap::new();
        for (name, value) in builtins::constants() {
            constants.insert(name.as_bytes().to_vec(), value);
        }
        Vm {
            out: Vec::new(),
            stack: Vec::new(),
            functions: HashMap::new(),
            constants,
            classes: Vec::new(),
            class_ids: HashMap::new(),
            max_depth: 1 << 16,
        }
    }

    /// The accumulated output buffer (mirrors `Interp::output`, so the
    /// differential harness reads both tiers the same way).
    pub fn output(&self) -> &[u8] {
        &self.out
    }

    /// Execute a compiled program; returns the process exit code.
    /// `Program` here is the *compiled* module (§3), not mago's `Program`.
    pub fn run(&mut self, prog: &Program) -> Result<i32, Thrown> {
        // Register this module's functions/classes into the VM tables, then
        // enter `main` as frame 0 and trampoline (§1.4).
        self.install(prog);
        match self.call_main(prog.main.clone()) {
            Ok(_value) => Ok(0),
            Err(Thrown::Exit(code)) => Ok(code), // exit/die: output kept, clean code
            Err(other) => Err(other),            // uncaught throw / refusal -> SAPI
        }
    }
}
```

The constant pool, function table, and class tables live **in the `Vm`**, never
in a `Value`. A `Value::Object` carries a `ClassId` (a `u32` index), so values
stay `'static` and detached from the engine, exactly as `value-model.md`
requires.

### 1.3 The dispatch skeleton

Registers hold `Value`s. Because `Value` clones are O(1) (`Rc` bump for
`Str`/`Array`, `Copy`-ish for scalars), an opcode reads its operands by cloning
them out of the register file *before* it touches `self` mutably — this sidesteps
the borrow conflict between "borrow a register" and "call `ops::`/a builtin that
takes `&mut self`". The clone is a bump, not a deep copy.

```rust
impl Vm {
    /// Run the frame on top of the stack until it returns, pushing/popping
    /// callee frames in place (no native recursion across PHP calls).
    fn run_top(&mut self) -> Result<Value, Thrown> {
        loop {
            // Re-borrow the top frame each step (indices, not held refs).
            let top = self.stack.len() - 1;
            let pc = self.stack[top].pc;
            // `code` is Rc<[Instr]>; clone the Rc handle, not the slice.
            let proto = self.stack[top].proto.clone();
            let instr = proto.code[pc];
            self.stack[top].pc = pc + 1;

            match instr {
                // ---- data movement -------------------------------------
                Instr::LoadConst { dst, k } => {
                    let v = proto.consts[k as usize].clone();
                    self.set(top, dst, v);
                }
                Instr::Mov { dst, src } => {
                    let v = self.get(top, src);
                    self.set(top, dst, v);
                }

                // ---- operators: pure delegation to ops:: ---------------
                Instr::Add { dst, a, b } => {
                    let (x, y) = (self.get(top, a), self.get(top, b));
                    let v = ops::add(&x, &y)?;        // '?' unwinds as a Thrown
                    self.set(top, dst, v);
                }
                Instr::Concat { dst, a, b } => {
                    let (x, y) = (self.get(top, a), self.get(top, b));
                    self.set(top, dst, ops::concat(&x, &y));
                }
                Instr::Cmp { dst, a, b, kind } => {
                    let (x, y) = (self.get(top, a), self.get(top, b));
                    self.set(top, dst, eval_cmp(kind, &x, &y)); // wraps ops::loose_* / strict_eq
                }
                // ... Sub/Mul/Div/Mod/Pow/Bitwise/Neg/Not/BitNot: one arm each,
                //     each calling the matching ops:: function.

                // ---- output --------------------------------------------
                Instr::Echo { src } => {
                    let v = self.get(top, src);
                    let bytes = phrust_values::conv::to_string_bytes(&v);
                    self.write_out(&bytes);
                }

                // ---- control flow (intra-proc) -------------------------
                Instr::Jmp { target } => self.stack[top].pc = target as usize,
                Instr::JmpIfFalse { cond, target } => {
                    if !phrust_values::conv::to_bool(&self.get(top, cond)) {
                        self.stack[top].pc = target as usize;
                    }
                }

                // ---- calls (inter-proc: manipulate the stack) ----------
                Instr::CallUser { .. } => {
                    // Build the callee Frame, push it, continue the loop with
                    // the callee on top. Return value is delivered by `Return`
                    // into the caller's `ret_dst` (§2.3).
                    self.enter_user_call(top, instr)?;
                }
                Instr::CallNative { dst, builtin, base, argc } => {
                    let args = self.collect_args(top, base, argc);
                    let out = (builtin.func)(self, &args)?; // self: &mut dyn Host
                    self.set(top, dst, out);
                }

                // ---- return: pop a frame, deliver to caller ------------
                Instr::Return { src } => {
                    let v = self.get(top, src);
                    let frame = self.stack.pop().unwrap();
                    match self.stack.last_mut() {
                        Some(caller) => caller.deliver_return(frame.ret_dst, v),
                        None => return Ok(v), // returned from `main`
                    }
                }

                // ---- exceptions (see §4) -------------------------------
                Instr::Throw { src } => {
                    let v = self.get(top, src);
                    return Err(self.make_throwable(v)); // unwound by run()'s driver
                }

                Instr::Refuse { feature, reason } => {
                    return Err(Thrown::unsupported(feature.to_string(), reason));
                }
                // ... remaining opcodes.
            }
        }
    }
}
```

Helpers `get`/`set` are register-file accessors on `self.stack[top].regs`; they
**auto-deref `Value::Ref` on read** (so most opcodes see through references for
free) and write-through a `Ref` slot when the destination CV holds one (§2.4).

---

## 2. Call frames and the call stack

### 2.1 The Frame

```rust
pub struct Frame {
    /// The compiled function body. `Rc` so recursion/closures share one copy.
    pub proto: Rc<Proto>,

    /// The flat register file: CVs (named PHP locals) in slots
    /// `[0 .. proto.n_locals)`, temporaries above, total `proto.n_registers`.
    /// A slot may hold `Value::Ref` to alias a caller slot (by-ref / `global`).
    pub regs: Vec<Value>,

    /// Instruction pointer into `proto.code`.
    pub pc: usize,

    /// Where this call's return value is written in the *caller's* registers.
    pub ret_dst: Reg,

    /// Unwind bookkeeping for `finally` (§4): a pending action to resume after
    /// the finally block completes.
    pub pending: Option<Unwind>,
}

pub type Reg = u16; // 65k registers/frame is plenty; widen to u32 only if needed
```

### 2.2 Call stack: explicit `Vec<Frame>`, not native recursion

**Recommendation: an explicit stack (`Vm::stack: Vec<Frame>`).** Reasons:

- **Deep PHP recursion** does not consume the Rust call stack, so a runaway PHP
  recursion raises a catchable PHP `Error` (a `max_depth` length check) instead
  of aborting the process on a native stack overflow.
- **Generators / fibers (future).** A suspended generator is just a detached
  `Frame` (its `regs` + `pc` are the entire resumable state). Native recursion
  cannot park a half-finished C stack; an explicit frame can be moved aside and
  resumed. We do not build generators now, but the explicit stack is what makes
  them tractable later.
- **JIT alignment.** The Cranelift tier will own its own activation records; a
  value-stack we already control is the natural interop boundary.

A `CallUser` opcode therefore does **not** call `run_top` recursively — it pushes
a `Frame` and lets the single top-level loop continue with the new top. `Return`
pops. The only native recursion is the outer driver that restarts `run_top`
after an exception unwinds past a frame (§4), bounded by `try` nesting, not by
PHP call depth.

### 2.3 Argument binding and return flow

The compiler assigns each declared parameter CV slot `i` the register `i`
(params occupy the lowest CV slots, in order). On a `CallUser`:

1. The caller has evaluated the arguments into a contiguous temp window
   `[base .. base+argc)`.
2. `enter_user_call` allocates the callee `Frame` with `regs` sized to
   `proto.n_registers`, scalars defaulting to `Value::Null`.
3. For each param `i`: bind `regs[i]` from arg `i` **by value** (a `Value`
   clone — Rc bump) for normal params, or **by reference** (see §2.4) when
   `proto.params[i].by_ref`. Missing trailing args take the param's compiled
   default (a tiny const-init sub-sequence, or a `Thrown::Error`
   `ArgumentCountError` when no default and not variadic). A variadic tail param
   collects the remaining args into a fresh `PhpArray`.
4. `ret_dst` on the callee records the caller register the result lands in.

`Return { src }` pops the callee and writes the value into
`caller.regs[ret_dst]` (`deliver_return`). Falling off the end of a function
body compiles to `Return` of `Value::Null`.

Named arguments (PHP 8) are resolved **at compile time** against the callee
proto's parameter names into positional slots where the callee is statically
known; the dynamic-callee case reorders at the call site using `proto.params`.

### 2.4 By-reference parameters and aliasing — reuse `Value::Ref`

A by-ref param aliases the caller's slot. We reuse `Value::Ref =
Rc<RefCell<Value>>` exactly as `value-model.md §4` intends, with **no new
machinery**:

- The compiler marks the argument expression for a by-ref param as needing an
  *lvalue slot*. At the call site the caller promotes the target CV to a shared
  slot: if `regs[v]` is not already a `Ref`, replace it in place with
  `Value::Ref(Rc::new(RefCell::new(old)))`; clone that `Rc` into the callee's
  param register. Now both frames' registers point at the same `RefCell`.
- Reads auto-deref (`get` borrows through the `RefCell`); writes to a `Ref`-typed
  register write *through* the cell (`*cell.borrow_mut() = v`) instead of
  replacing the register, so the alias stays live. Plain (non-`Ref`) registers
  are replaced wholesale — ordinary by-value semantics.
- `global $x;` and `static $x;` are the same promotion against the global scope
  frame / a per-proto static cell, respectively. COW value types still isolate:
  binding a `Ref` to an `Array` and mutating it is explicit shared mutation,
  while a plain by-value `Array` arg is `make_mut`-isolated on first write.

Because `Ref` is just a slot discipline (not a PHP type), `gettype`, comparisons,
and dumps already see through it (the existing `deref` in `ops::`/builtins), so
no opcode special-cases it beyond `get`/`set`.

---

## 3. Compile-once: AST → Proto, cached

User functions (and `main`, and later methods) are compiled to bytecode **once**,
the first time the declaring statement is reached, and cached as `Rc<Proto>` in
the VM's function table. They are never re-walked.

```rust
/// One compiled function body. Immutable after compilation; shared by `Rc`.
pub struct Proto {
    pub name: Rc<[u8]>,                 // for stack traces / error messages
    pub n_params: u16,
    pub n_locals: u16,                  // CV count (named locals known statically)
    pub n_registers: u16,               // locals + temporaries to allocate
    pub params: Vec<ParamInfo>,         // by_ref / variadic / default init
    pub local_names: Vec<Rc<[u8]>>,     // CV slot -> PHP name (debug, `global`)
    pub code: Rc<[Instr]>,              // the instruction stream
    pub consts: Vec<Value>,             // literal pool (owned, 'static)
    pub handlers: Vec<Handler>,         // exception table (§4)
}

pub struct ParamInfo {
    pub by_ref: bool,
    pub variadic: bool,
    pub default: Option<DefaultInit>,   // const value, or a short init opcode range
}

/// The compiled module produced from one `ParsedFile`.
pub struct Program {
    pub main: Rc<Proto>,                        // top-level statements
    pub functions: HashMap<Vec<u8>, Rc<Proto>>, // lowercase name -> body
    pub classes: Vec<ClassEntry>,               // phase 7
}

pub struct Compiler<'a> { /* arena-borrowing walk state: reg allocator, ... */ }

impl<'a> Compiler<'a> {
    /// Lower a parsed file to a module. Refusals (eval/include/backticks/
    /// (unset)-cast, and `is_refused()` names reached structurally) are emitted
    /// as `Instr::Refuse`/compile errors HERE, never left to runtime.
    pub fn compile(pf: &'a ParsedFile) -> Result<Program, Thrown>;
}
```

- The compiler is a single pass over the mago CST (same `Statement`/`Expression`
  enums the oracle walks), borrowing `'arena` and copying literals *out* into
  `consts` as owned `Value`s on production — so a `Proto` is `'static` and the
  arena/`ParsedFile` can be dropped after compilation. (Values never pin the
  arena; only the walk does.)
- **Register allocation** is a simple per-proto scheme: CV slots are assigned to
  named `$vars` first (fixed `0..n_locals`), temporaries are allocated from a
  high-water bump counter and freed at expression-statement boundaries. This is
  intentionally SSA-friendly for the future JIT without demanding full SSA now.
- `Statement::Function` declarations register their `Rc<Proto>` into
  `Program::functions` (and, at run time, into `Vm::functions` when the
  declaration executes — conditional function definitions are handled by an
  install opcode). Redeclaration raises the PHP fatal.
- Classes/methods (phase 7) compile each method to its own `Proto` under a
  `ClassEntry`; the `ClassId` indexes `Vm::classes`.
- **Refusals are structural.** `Construct::Eval`, `Construct::Include*`/`Require*`,
  `CompositeString::ShellExecute` (backticks), and `UnaryPrefixOperator::UnsetCast`
  compile to a refusal at compile time; calls to `is_refused()` names likewise.
  `eval` is intentionally *not* a VM feature — there is no opcode that interprets
  a string as code, now or later.

---

## 4. Exception unwinding at runtime

Real throws and engine refusals both ride `Err(Thrown)` (the same decision the
oracle made in `README.md §3`), so Rust's `?` does the per-opcode unwinding for
free. Cross-frame unwinding is driven by a per-proto **exception table**, not by
scanning instructions.

```rust
pub struct Handler {
    pub start: u32,                 // protected region [start, end) in `code`
    pub end: u32,
    pub catches: Vec<CatchArm>,     // in source order; first type match wins
    pub finally: Option<u32>,       // finally entry pc (runs on all exits)
}
pub struct CatchArm {
    pub classes: Vec<Rc<[u8]>>,     // `catch (A|B $e)` -> ["A","B"]
    pub var_reg: Option<Reg>,       // CV slot for `$e` (None: `catch (A)`)
    pub target: u32,                // handler body pc
}
```

The driver that owns `run_top` turns an `Err(Thrown)` into a search:

```rust
impl Vm {
    fn run_driver(&mut self) -> Result<Value, Thrown> {
        loop {
            match self.run_top() {
                Ok(v) => return Ok(v),
                Err(t) => match self.unwind(t)? {
                    // `unwind` either resumed execution at a handler (loop again
                    // to re-enter run_top) or re-raised past `main`.
                    Resume::Caught => continue,
                    Resume::Exit(code) => return Err(Thrown::Exit(code)),
                },
            }
        }
    }

    /// Walk frames top-down looking for a handler whose protected range covers
    /// the throwing pc and whose catch type matches.
    fn unwind(&mut self, thrown: Thrown) -> Result<Resume, Thrown> {
        loop {
            let Some(frame) = self.stack.last_mut() else {
                return Err(thrown); // escaped main -> SAPI (uncaught / refusal / exit)
            };
            match classify(&thrown) {
                // Exit / Unsupported are NOT catchable PHP Throwables: pop every
                // frame and hand them straight to the SAPI.
                Caught::NotCatchable => { self.stack.clear(); return Err(thrown); }
                Caught::Throwable(class) => {
                    if let Some(h) = find_handler(&frame.proto, frame.pc, class, &self.classes) {
                        if let Some(arm) = h.matching_catch(class, &self.classes) {
                            if let Some(reg) = arm.var_reg {
                                frame.regs[reg as usize] = thrown_to_value(&thrown);
                            }
                            frame.pc = arm.target as usize; // enter catch body
                            return Ok(Resume::Caught);
                        }
                        if let Some(fin) = h.finally {
                            // No catch matched but a finally guards this region:
                            // run finally, remembering to re-raise afterwards.
                            frame.pending = Some(Unwind::Rethrow(thrown.clone()));
                            frame.pc = fin as usize;
                            return Ok(Resume::Caught);
                        }
                    }
                    // No handler in this frame: pop and continue unwinding.
                    self.stack.pop();
                    // (A finally on the popped frame is run before the pop via
                    //  the Unwind::Rethrow path above.)
                }
            }
        }
    }
}
```

Key points:

- **`finally` semantics** use `Frame::pending: Option<Unwind>`. Entering a
  `finally` block (whether on normal fall-through, a `return` crossing it, a
  `break`/`continue` crossing it, or an in-flight throw) records what to resume
  when the block ends: `Unwind::Normal`, `Unwind::Return(Value)`,
  `Unwind::Control(Flow)`, or `Unwind::Rethrow(Thrown)`. The last opcode of a
  finally region is a `FinallyEnd` that pops `pending` and performs it — so a
  `return` inside `finally` correctly overrides a pending throw, matching PHP.
- **Catch matching** is an `instanceof`-style check against the thrown object's
  class and its parents/interfaces (`Vm::classes`). Until OOP lands,
  `Thrown::Error{class,..}` carries a `&'static str` class name
  (`DivisionByZeroError`, `TypeError`, …) and matching is name/hierarchy lookup
  against the built-in exception hierarchy.
- **`Thrown::Exit(code)`** is not an error and not catchable: `unwind` clears the
  stack and returns it so `Vm::run` maps it to the exit code, **keeping already
  written output** (the `out` buffer is never discarded on `Exit`).
- **`Thrown::Unsupported{feature,reason}`** is also not catchable: it unwinds
  every frame and reaches the SAPI, which renders it with the `PHRUST_UNSUPPORTED`
  marker — distinct from PHP's "undefined function". A refusal can therefore
  never be swallowed by a user `try/catch`.

---

## 5. Builtins and operators

### 5.1 Builtins: reuse `registry()` / `Host`

The VM **is** the `Host` (its `write_out` appends to `out`). A `CallNative`
opcode is emitted whenever the compiler sees a call whose name resolves to
`builtins::lookup(name)` and is not a user function:

1. Arguments are evaluated into a contiguous `[base .. base+argc)` window.
2. Arity is checked against `Builtin::{min_args, max_args}` (the compiler can
   pre-check fixed-arity calls and emit the `ArgumentCountError` inline; variadic
   stays a runtime check), raising `ArgumentCountError` otherwise.
3. The handler calls `(builtin.func)(self, &args)` — `self` coerces to
   `&mut dyn Host`, so builtins keep operating on owned `Value`s with zero
   knowledge of frames, registers, or the arena.
4. The `Result<Value, Thrown>` flows back exactly like a user return: `Ok` into
   `dst`, `Err` into the unwinder.

`is_refused()` names are rejected at **compile** time (a `Refuse` opcode / compile
error), so a refused builtin never reaches dispatch. By-ref builtins (future
`settype`-style) use the builtins-crate's ref entry point; the VM passes the
`Value::Ref` slot rather than a cloned value.

### 5.2 Operators: one opcode per `ops::` function

Every arithmetic/comparison/bitwise/concat/unary opcode is a wrapper:
`Instr::Add` → `ops::add`, `Sub` → `ops::sub`, … `Concat` → `ops::concat`,
`Cmp{kind}` → `ops::loose_compare`/`loose_eq`/`strict_eq`/`spaceship`, `Neg` →
`ops::negate`, `BitNot` → `ops::bitwise_not`, `Bit{op}` → `ops::bitwise`. The VM
contributes only operand fetch, `Thrown` propagation via `?`, and `dst` write.
Short-circuit `&&`/`||`/`??` are **not** opcodes — the compiler lowers them to
`JmpIfFalse`/`JmpIfTrue`/`JmpIfNotNull` + `Mov`, so only the taken side executes,
preserving PHP's lazy evaluation. `inc`/`dec` reuse the oracle's `inc_dec_value`
rule (promoted to a shared helper).

This guarantees the VM and oracle are **semantically identical by construction**
on all operators — they call the same functions — so any differential mismatch
localizes to lowering (compiler) bugs, never to duplicated semantics.

---

## 6. Differential strategy

Both tiers expose the **same entrypoint shape**, so the harness can run
`php` vs `vm` vs `oracle` on identical inputs and compare bytes + exit code.

```rust
/// One engine's result for a single source file.
pub struct Outcome {
    pub stdout: Vec<u8>,
    pub exit: i32,
    /// Set when the run ended on an uncaught throw / refusal (rendered to
    /// stderr by the SAPI; excluded from the stdout byte-compare).
    pub error: Option<Thrown>,
}

/// Implemented by BOTH the VM and the oracle `Interp`.
pub trait Engine {
    fn execute(pf: &ParsedFile) -> Outcome;
}

impl Engine for Vm {
    fn execute(pf: &ParsedFile) -> Outcome {
        let prog = match Compiler::compile(pf) {
            Ok(p) => p,
            Err(t) => return Outcome { stdout: Vec::new(), exit: 255, error: Some(t) },
        };
        let mut vm = Vm::new();
        let r = vm.run(&prog);
        finish(vm.output(), r) // -> Outcome (exit code + optional error)
    }
}

impl Engine for Interp { /* wraps the existing Interp::run(&ParsedFile) */ }
```

- `Vm::run(&Program) -> Result<i32, Thrown>` is the native signature the task
  calls for; `execute` is the thin, uniform shell the harness drives both tiers
  through (compile + run + package), mirroring `Interp::output()` / `Interp::run`.
- **The runner** (in `phrust-cli`, extending the existing `tests/diff.rs`): glob
  `tests/golden/*.php` **and** `.phpt` cases; for each, produce three `Outcome`s —
  real `php` 8.5.8 via `Command::new("php")`, `Vm::execute`, `Interp::execute`.
  Assert `php.stdout == vm.stdout` and `php.exit == vm.exit` (the product gate),
  and additionally `oracle.stdout == vm.stdout` on the oracle's covered subset
  (a cheap, `php`-free cross-check that localizes regressions to a tier). A
  mismatch reports the fixture, first differing byte offset, and all three
  lossy-rendered streams.
- `.phpt` support parses the standard `--TEST--/--FILE--/--EXPECT[F]--` sections;
  `EXPECTF` patterns compare against the documented `%d/%s/...` wildcards. Cases
  that hit refused features assert the `PHRUST_UNSUPPORTED` marker rather than a
  byte match.
- Because operators and builtins are *shared code* (§5), the oracle is a genuine
  independent check only of the **compiler + VM loop**, which is exactly where VM
  bugs live — the highest-value thing to diff against.

---

## 7. Forward compatibility (JIT) — guardrails, not work

Choices made here so the Cranelift tier slots in without a rewrite, none of which
we build now:

- **Typed, 3-address opcodes over a flat register file** map directly to SSA
  values / Cranelift `Value`s; `Proto::n_registers` is the frame's value count.
- **`Rc<Proto>` + `Rc<[Instr]>`** mean a proto can be handed to a JIT backend and
  cached (compiled-code pointer stored beside the `Proto`) without touching the
  interpreter.
- **Explicit call stack + value stack** are the interop boundary with JIT-ted
  frames (a JIT frame and an interpreted frame can coexist on `Vm::stack`).
- **`ops::` as the single source of operator truth** means the JIT can either
  call the same functions (baseline) or inline specialized versions (optimized)
  while staying bit-identical to the interpreter.
- **No opcode interprets a string as code** — keeping `eval` refused means the
  compiled set is closed and statically analyzable, which the JIT relies on.

---

## 8. Opcode inventory (first cut)

Grouped; operands are `Reg` (register index) or `u32` (const-pool / jump / proto
index). This is the phase-by-phase lowering target, not a frozen ISA.

- **Move/const:** `LoadConst`, `Mov`, `LoadConstName` (user/engine constant),
  `Null dst`.
- **Arithmetic/bitwise/unary:** `Add Sub Mul Div Mod Pow`, `BitAnd BitOr BitXor
  Shl Shr`, `Neg Not BitNot`, `Inc Dec` (→ `ops::` + `inc_dec_value`).
- **Compare:** `Cmp{Eq,Ne,Identical,NotIdentical,Lt,Le,Gt,Ge,Spaceship}`.
- **Concat/output:** `Concat`, `Echo`, `Print` (echo + yields `int(1)`).
- **Control flow:** `Jmp`, `JmpIfTrue`, `JmpIfFalse`, `JmpIfNotNull` (for `??`),
  `Switch`/`Match` lowered to compare+jump chains (`Match` → strict-eq +
  `UnhandledMatchError`).
- **Vars/arrays:** `FetchDim`/`AssignDim` (array read/write + autovivify),
  `AppendDim` (`$a[] =`), `Isset`, `Empty`, `Unset`, `MakeArray`,
  `Spread`, `ForeachInit`/`ForeachNext` (iterator over a `PhpArray`).
- **Calls:** `CallUser`, `CallNative`, `CallDynamic` (callable value), `InitArg`/
  `SendRef` (argument staging, by-ref promotion), `Return`.
- **Refs/scope:** `MakeRef`, `BindGlobal`, `BindStatic`.
- **Exceptions:** `Throw`, `FinallyEnd` (consume `Frame::pending`); `try` regions
  are described by `Proto::handlers`, not opcodes.
- **Refusal:** `Refuse { feature, reason }` (emitted by the compiler only for
  structurally-refused nodes that survive to a reachable position).
- **OOP (phase 7):** `New`, `FetchProp`/`AssignProp`, `CallMethod`,
  `CallStatic`, `InstanceOf`, `Clone`.

---

## 9. Pitfalls specific to the VM

- **Deref discipline in `get`/`set`.** `get` auto-derefs `Value::Ref`; `set`
  must write *through* a `Ref`-typed slot, not overwrite it, or aliases silently
  break. Every by-ref test depends on this one rule.
- **COW still holds.** Registers hold `Value`s; mutating an `Array`/`Str`
  register goes through `make_mut` on that register's value, never on a cloned-out
  `Rc`. `$b = $a; $b[0] = 1;` must not touch `$a` — the `Mov` shares the `Rc`,
  the `AssignDim` triggers COW.
- **Clone-before-call.** Opcodes fetch operands by cloning out of the register
  file before calling `ops::`/builtins (both take refs while `self` is borrowed
  mut); the clone is an `Rc` bump, so this is cheap, not a deep copy.
- **Stack-depth guard, not native overflow.** Check `self.stack.len() <
  max_depth` on every `CallUser`; exceed → catchable PHP `Error`.
- **Two float precisions survive lowering.** `Echo`/`Concat`/`(string)` use
  `fmt::float_echo` (precision 14); `var_dump`/`var_export` use
  `fmt::float_serialize` (−1). The compiler must route to the right one — the
  opcode does not choose.
- **Refusals cannot be lowered into catchable form.** `Construct::Eval` and the
  rest compile to a refusal; never emit a `Throw` for them (a user `catch`
  would swallow it). `eval` has no opcode, by design and per the user's explicit
  instruction.
