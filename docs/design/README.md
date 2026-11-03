# rhp — implementation spec (scaffold contract)

> **Historical document — not the current design.** This is the original
> scaffold contract for rhp's first engine, a tree-walking interpreter in an
> `rhp-runtime` crate. When the bytecode compiler + VM replaced it, the
> tree-walker was kept for a while as a differential "oracle", then removed
> (2026-11). Nothing below is authoritative any more: the crate layout, the
> `Interp` type and the refusal list (`include`/`require` and `eval` now run)
> are all out of date. For the current design read the project
> [README](../../README.md), [`bytecode.md`](./bytecode.md) (compiler + VM),
> [`sapi.md`](./sapi.md) (request layer), [`extensions.md`](./extensions.md),
> [`libraries.md`](./libraries.md) (the C libraries rhp links) and, for exact
> php behaviour, [`php-semantics.md`](./php-semantics.md) and
> [`value-model.md`](./value-model.md).

rhp is a PHP execution engine in Rust: a tree-walking interpreter over the
mago-syntax CST with a CLI SAPI, targeting **PHP 8.5 semantics** (ground truth:
the reference `php`, 8.5.12 — see `crates/rhp-builtins/src/version.rs`). This document is the single authoritative build contract. It
synthesizes the four design docs in this directory — read them for the "why":

- [`ast-surface.md`](./ast-surface.md) — every `Statement`/`Expression` variant, operators, casts, high-risk nodes, phase→node checklist.
- [`value-model.md`](./value-model.md) — the owned `Value` enum, `PhpStr`/`PhpArray`/`PhpRef`, COW vs aliasing, key normalization.
- [`php-semantics.md`](./php-semantics.md) — byte-exact PHP 8.5.8 output/number/comparison/cast rules; the `precision=14` vs `serialize_precision=-1` split.
- [`builtins-min.md`](./builtins-min.md) — minimal builtin set, `NativeFn` signature, refused functions.
- [`sapi.md`](./sapi.md) — the request layer (`rhp-sapi`): the `Sapi` trait and `RequestInfo` an embedding server provides, the request lifecycle, one `Vm` per thread, and the road to `-S`, server embedding and FastCGI.

Non-negotiable invariant threaded through everything: **the arena holds the AST
(borrowed `'arena`); the runtime holds only OWNED `'static` values.** A value
copied out of the arena the moment it is produced must outlive the statement
that produced it.

---

## 1. Workspace & crate layout

Cargo workspace, `resolver = "2"`, five library/binary crates under `crates/*`.
Strict one-directional dependency graph (no cycles):

```
          rhp-cli  (bin: the SAPI / entry point, test runner)
              |
              v
        rhp-runtime  (tree-walking interpreter, scopes, Flow)
          /        \
         v          v
 rhp-builtins   rhp-frontend   (mago wrapper, owns arenas)
         |
         v
   rhp-values   (Value / PhpStr / PhpArray / PhpRef — zero PHP-logic deps)
```

Edges (each crate depends only on what is below/beside it):

| crate | depends on |
|---|---|
| `rhp-values` | `bstr`, `indexmap`, `rustc-hash` (`ordered-float` reserved, unused) |
| `rhp-frontend` | `mago-syntax`, `mago-allocator`, `mago-database`, `mago-span` |
| `rhp-builtins` | `rhp-values` |
| `rhp-runtime` | `rhp-values`, `rhp-frontend`, `rhp-builtins` |
| `rhp-cli` | `rhp-runtime` (+ `rhp-frontend`, `rhp-values` re-exports) |

`rhp-builtins` must **not** depend on `rhp-frontend` (builtins never touch
the arena/AST — they operate on owned `Value`s only). `rhp-runtime` is the
only crate that knits AST + values + builtins together.

### 1.1 `rhp-values` — the data model

The zval analogue. No arena lifetime anywhere; everything is `'static` and
cheaply cloneable. Public types (see value-model.md §1–5):

```rust
#[derive(Clone)]                               // Clone is the engine workhorse
pub enum Value {
    Null, Bool(bool), Int(i64), Float(f64),    // unboxed scalars (f64, NOT OrderedFloat)
    Str(PhpStr),                               // Rc<BString>, COW
    Array(PhpArray),                           // Rc<ArrayData>, COW value semantics
    Object(ObjectHandle),                      // Rc<RefCell<ObjectData>>, aliasing (phase 7)
    Ref(PhpRef),                               // Rc<RefCell<Value>>, aliasing marker
}

pub struct PhpStr(Rc<BString>);                // make_mut() -> &mut BString  (COW)
pub enum ArrayKey { Int(i64), Str(PhpStr) }    // #[derive(Clone,PartialEq,Eq,Hash)]
pub struct ArrayData {
    pub entries: IndexMap<ArrayKey, Value, FxBuildHasher>,
    pub next_int: Option<i64>,                 // high-water append counter
}
pub struct PhpArray(Rc<ArrayData>);            // make_mut() -> &mut ArrayData (COW)
pub type PhpRef = Rc<RefCell<Value>>;
pub type ClassId = u32;
pub type ObjectHandle = Rc<RefCell<ObjectData>>;   // phase 7
```

Also owns: `ArrayKey::from_value` (key normalization, value-model.md §3b — the
`(string)(int)s === s` canonical-int-string rule, bool/null/float coercion),
`float_to_int(f64) -> i64` (PHP `zend_dval_to_lval`, toward-zero, NaN/INF→0,
reused by `(int)` casts and float keys), and the `next_int` update rule
(`None => k+1`, `Some(n) => n.max(k+1)`, never lowered by unset, `i64::MAX`
disables append). This crate has **no knowledge of the AST or of PHP operator
semantics** — it is pure storage + key rules.

### 1.2 `rhp-frontend` — the parser wrapper (ALREADY SCAFFOLDED)

Owns the arena and hides mago's self-referential borrow. Already present at
`crates/rhp-frontend/src/lib.rs`: `ParsedFile` bundles a `Box<LocalArena>`
with the `&'arena Program` (stored as a raw pointer, handed back only tied to
`&self`), plus `source`, `logical_name`. Public surface:

```rust
pub use mago_syntax::cst;                               // Statement, Expression, ...
pub struct ParsedFile { /* arena + program + source */ }
impl ParsedFile {
    pub fn parse(logical_name: impl Into<Vec<u8>>, source: impl Into<Vec<u8>>) -> Self;
    pub fn program(&self) -> &Program<'_>;              // borrow tied to &self
    pub fn source(&self) -> &[u8];
    pub fn logical_name(&self) -> &[u8];
    pub fn has_parse_errors(&self) -> bool;             // program().has_errors()
}
```

Entry point reminder (mago 1.58.2, verified in-tree):
`parse_file_content<'arena, A: Arena>(&'arena A, FileId, &[u8]) -> &'arena Program<'arena>`;
the arena type is `mago_allocator::LocalArena`. The runtime never sees
`LocalArena` directly — it holds a `ParsedFile` and walks `.program().statements`.

### 1.3 `rhp-builtins` — native function + constant registry

```rust
pub enum Thrown {                                       // unwinds the interpreter
    Error { class: &'static str, message: String },     // real Throwable (phase 5 catch)
    Unsupported { function: &'static str, reason: &'static str },  // RHP_UNSUPPORTED
}
pub type NativeFn = fn(&mut dyn Host, &[Value]) -> Result<Value, Thrown>;
pub struct Builtin { pub name: &'static str, pub func: NativeFn,
                     pub min_args: usize, pub max_args: Option<usize> }
pub fn registry() -> &'static HashMap<&'static str, Builtin>;   // case-insensitive lookup
pub fn constants() -> &'static [(&'static str, Value)];         // PHP_EOL, PHP_INT_MAX, ...
pub fn is_refused(name: &str) -> Option<&'static str>;          // extract/compact/create_function
```

To avoid a dependency on `rhp-runtime`, builtins talk to the interpreter
through a narrow `Host` trait defined here and implemented by the runtime
(`fn write_out(&mut self, bytes: &[u8])` for the output sink, plus a precision
getter). By-ref builtins (`settype`) use a small allowlist + a `NativeRefFn`
second entry point (builtins-min.md §1.4). Minimal set: `gettype`/`settype`/
`is_*`/`is_numeric`, `intval`/`floatval`/`strval`/`boolval`, `var_dump`/`print_r`/
`var_export`, `strlen`/`count`/`str_repeat`/`implode`/`explode`, `sprintf`/
`printf`. Refused names resolve to `Thrown::Unsupported`.

### 1.4 `rhp-runtime` — the tree-walker

The core. Holds the interpreter state and the eval functions. Key public types:

```rust
pub struct Interp {
    pub out: Vec<u8>,            // or a Box<dyn Write>; the single stdout sink
    globals: Scope,              // name -> slot
    functions: HashMap<Vec<u8>, FnDef>,   // user functions (case-insensitive key)
    constants: HashMap<Vec<u8>, Value>,   // builtin + user const
    // phase 7: classes: HashMap<Vec<u8>, ClassId>, class_table: Vec<ClassEntry>
}
pub enum Flow {                  // control-flow signal — see §3
    Normal, Break(u32), Continue(u32), Return(Value), Throw(Value),
}
pub struct Scope { vars: HashMap<Vec<u8>, Slot> }   // Slot = a place holding a Value
pub type ExecResult = Result<Flow, Thrown>;         // stmt exec
pub type EvalResult = Result<Value, Thrown>;        // expr eval

impl Interp {
    pub fn new() -> Self;                            // installs builtin constants
    pub fn run(&mut self, pf: &ParsedFile) -> Result<i32, Thrown>;   // -> exit code
}
```

`rhp-runtime` implements `rhp_builtins::Host` for `Interp` (writes into
`out`, exposes precision). It owns: statement exec (`exec_stmt`, `exec_block`),
expression eval (`eval_expr`), lvalue resolution (`eval_lvalue` → a `Slot`),
the operator tables (arith/compare/concat/bitwise with PHP coercion), the
stringify path (precision-14 `echo`/concat formatting), and the scalar casts.

### 1.5 `rhp-cli` — the SAPI / binary

`cargo run -p rhp-cli -- script.php`. Reads the file, `ParsedFile::parse`,
bails with a diagnostic if `has_parse_errors()`, else `Interp::run`, flushes
`out` to real stdout, exits with the returned code (default 0; `exit($n)` sets
it; uncaught `Thrown` → stderr fatal + nonzero). Also hosts the differential
test runner (§4).

---

## 2. Arena ownership strategy

**One arena per source file, kept alive for as long as that file's AST is
walked.** mago parses into a `LocalArena`; the returned `Program` and every
`Statement`/`Expression`/`&[u8]` borrows it. The runtime copies bytes *out* into
owned `Value`s on production, so runtime data never pins the arena — but the AST
being walked does.

- **Phase 1 (single file):** `rhp-cli` creates one `ParsedFile` (which boxes
  the `LocalArena`). The `Interp::run(&pf)` borrow lasts the whole execution.
  Values produced during the run are `'static` and survive `pf` being dropped at
  process exit.
- **The self-reference is already solved** in `rhp-frontend::ParsedFile`:
  `Box<LocalArena>` + a raw `*const Program<'static>` reborrowed only as
  `&self`-tied. Nothing else in the tree uses `unsafe`; treat `ParsedFile` as
  the sole owner of that trick.
- **Forward path to include/require:** the interpreter will hold a
  `Vec<ParsedFile>` (or `HashMap<PathBuf, ParsedFile>` for `_once`), **one arena
  per included file, all kept alive for the whole run**. `include`/`require`
  parse a new `ParsedFile`, push it into that collection, and walk its program;
  because each file keeps its own arena, ASTs from different files coexist. No
  shared arena, no cross-file `'arena` borrows — values flow between files as
  owned `Value`s only. (Include/require are refused/deferred in phases 1–4; this
  is the shape they slot into.)

Rule to never break: **do not store an `&'arena` anything in `Interp`, `Scope`,
`Value`, or a builtin.** The only arena borrow in flight is the `&ParsedFile`
threaded through the exec/eval call stack.

---

## 3. Control-flow signal enum

Statement execution returns `Result<Flow, Thrown>`. `Flow` carries non-error
control transfer **up** the exec stack; `Thrown` (the `Err` arm) carries real
PHP throws + engine refusals and unwinds the same way Rust `?` does.

```rust
pub enum Flow {
    Normal,              // fell off the end; keep going
    Break(u32),          // break N  — unwind N loop/switch levels
    Continue(u32),       // continue N
    Return(Value),       // return expr;  (None => Return(Value::Null))
    Throw(Value),        // throw expr;   (the thrown object/value)
}
```

Threading rules:

- `exec_stmt` / `exec_block` iterate statements; after each, **if the result is
  not `Flow::Normal`, stop and propagate it** (`match flow { Normal => continue, other => return Ok(other) }`).
- **Loops** (`while`/`do`/`for`/`foreach`) inspect the body's `Flow`:
  `Break(1)`/`Continue(1)` are consumed here (break exits, continue→next iter);
  `Break(n)`/`Continue(n)` with `n>1` decrement and re-propagate; `Return`/`Throw`
  propagate unchanged. **`switch` counts as one level** for break/continue.
- `Return(v)` propagates up to the enclosing function call boundary, which
  unwraps it to the call's value. `Return` escaping the top level ends the script.
- Two options for `Throw`: it may ride the `Flow::Throw` variant **or** the
  `Err(Thrown)` arm. **Decision: use `Err(Thrown)` for throws and refusals**, so
  `?` does the unwinding for free, `try/catch` is a `match` at the catch site,
  and `finally` runs on both the `Ok`/`Err` paths. `Flow::Throw` is kept in the
  enum for symmetry/throw-as-expression but the primary carrier is `Thrown`.
  (`Thrown::Error{class,message}` is what a `catch (SomeType $e)` matches on.)
- Expression eval returns `Result<Value, Thrown>` (no `Flow` — expressions don't
  break/return), except `throw` as an expression yields `Err(Thrown)`.

---

## 4. Phase-1 acceptance criterion (testable, byte-exact)

**Definition of done for the scaffold:** a differential runner discovers every
`tests/golden/*.php`, runs it under both `rhp` and the real `php` binary, and
asserts **identical stdout bytes and identical exit code**. Phase 1 passes when
the mixed-inline-HTML + echo/print + scalar/arithmetic/concat/comparison subset
matches byte-for-byte.

### 4.1 Required inputs (must match `php` exactly)

The 21 existing golden pairs already cover the number/comparison/cast/dump
surface. Phase 1 must at minimum pass this core subset plus new inline-HTML pairs:

- **Output & inline HTML:** text before `<?php`, text between `?> ... <?php`,
  trailing text after `?>`, the `?>`-swallows-one-newline rule, `<?= expr ?>`
  short-echo, `echo a, b, c;` comma list, `print` returning `int(1)`. A shebang
  first line must NOT print.
- **Scalar literals:** `int` (decimal/hex/octal/binary, `_` separators),
  `float` (incl. `1e20`, `1.5e-10`, `-0.0`), `string` (single/double quoted,
  escapes), `true`/`false`/`null`, overflow literal → float.
- **Arithmetic:** `+ - * / % ** `, int-overflow→float, `/` int-vs-float result,
  `intdiv`, modulo sign-of-dividend, `**` right-assoc & `-2**2 == -4`.
- **Concat:** `.` with the precision-14 float form (`echo "r=".(0.1+0.2)` → `r=0.3`).
- **Comparison:** PHP 8 loose `==` (`0 == "a"` is **false**), `===`, `<=>`,
  string-vs-numeric ordering, `null`/`""`/`[]` equalities.
- **Casts:** `(int)`/`(float)`/`(string)`/`(bool)` per php-semantics.md §6.

Covered already by: `echo_int`, `echo_float_formats`, `echo_bool_null`,
`int_overflow`, `intdiv_mod`, `power_operator`, `bitwise`, `comparison_php8`,
`spaceship`, `string_ordering`, `int_casts`, `float_casts`, `concatenation`,
`bool_casts`, `numeric_string_arith`, `is_numeric_cases`, `gettype_names`,
`var_dump_scalars`, `var_dump_nested`, `print_r_nested`, `var_export_nested`.
**Add** a handful of inline-HTML/echo-tag pairs (`inline_html`, `echo_tag`,
`echo_comma_list`, `print_returns_one`).

### 4.2 The differential runner

In `rhp-cli` (an integration test `tests/diff.rs`, or a `cargo test` harness):

1. Glob `tests/golden/*.php`. For each `NAME.php`:
2. Run rhp: `Interp::run(ParsedFile::parse(name, bytes))`, capture `out` + exit code.
3. Run reference: `std::process::Command::new("php").arg(path)`, capture stdout + exit code.
4. **Assert `rhp.stdout == php.stdout` (bytes) and `rhp.code == php.code`.**
   (Stderr is excluded — fixtures avoid fatal/warning cases so bytes are
   deterministic and path-independent; the `.out` files already hold exactly
   `php NAME.php` stdout.)
5. Prefer asserting against the committed `NAME.out` when present (no `php`
   needed in CI); fall back to invoking `php` to (re)generate/verify. Report each
   mismatch as a unified diff of the two byte streams.

A failing case prints: fixture name, first differing byte offset, expected vs
actual (lossy-rendered). Green runner = phase 1 done.

---

## 5. Ordered scaffold checklist (keep `cargo` green at every step)

Each step compiles and tests pass before the next.

1. **Workspace skeleton.** Confirm the five crates exist with correct deps
   (`rhp-frontend` is done). Add empty `rhp-values`, `rhp-builtins`,
   `rhp-runtime`, `rhp-cli` with `lib.rs`/`main.rs` stubs. `cargo build`
   green. Wire `rustc-hash` into `rhp-values`.
2. **`rhp-values`.** `Value`, `PhpStr`, `ArrayKey`, `ArrayData`, `PhpArray`,
   `PhpRef`. Implement `ArrayKey::from_value`, `float_to_int`, `next_int` update,
   `PhpStr`/`PhpArray` `make_mut`. Unit-test key normalization against the
   value-model.md §3b table. No PHP logic beyond keys.
3. **Stringify + casts (in `rhp-runtime`).** `to_php_string` (echo/concat
   path, **precision 14**, `1.0E+20` sci form, `true→"1"`, `false/null→""`),
   `to_bool`, `to_int` (via `float_to_int`), `to_float`, `to_number`. These are
   the backbone of everything. Unit-test against php-semantics.md §1/§6/§8.
4. **`Interp` + output sink + echo/print.** `exec_stmt` for `OpeningTag`,
   `ClosingTag`, `Inline` (print verbatim; skip `Shebang`), `Noop`, `Echo`,
   `EchoTag`, `Construct::Print`, `Expression` statement. `eval_expr` for
   `Literal`, `Parenthesized`. `Interp::run` drives `program().statements`.
   **Wire the refused set now** (eval/backticks/`(unset)` cast structurally;
   extract/compact/create_function via registry) so unsupported code fails loudly.
5. **CLI + differential runner.** `rhp-cli` main + `tests/diff.rs` (§4).
   `inline_html`/`echo`/`print` goldens go green. This is the first real
   acceptance gate.
6. **Operators & more expressions.** `Binary` (arith with overflow→float,
   compare with PHP 8 loose rules, `<=>`, concat, bitwise, short-circuit
   `&&`/`||`/`??`), `UnaryPrefix` (ops + the 4 supported casts + deprecated
   aliases + `(unset)`→fatal), `UnaryPostfix`, `Conditional`/elvis. Variables
   (`Variable::Direct`), `Assignment` (`=` + compound). All phase-1 number/
   comparison/concat goldens go green.
7. **Builtins (`rhp-builtins`).** Registry + `Host` trait + arity dispatch.
   Implement in builtins-min.md §5 order: `var_dump`/`gettype`/`is_*`/constants,
   then `intval`/`floatval`/`strval`/`boolval`/`settype`/`is_numeric`, then
   `strlen`/`count`/`str_repeat`/`implode`/`explode`, then `sprintf`/`printf`/
   `print_r`/`var_export`. `var_dump`/`print_r`/`var_export` use
   **serialize_precision = -1** (shortest round-trip), distinct from echo's 14.
   The dump/export goldens go green.
8. **Control flow.** `Flow` enum; `If`, `While`, `DoWhile`, `For`, `Switch`
   (fall-through + `==`), `Match` (`===`, UnhandledMatchError), `Break`/
   `Continue` with levels, `Return`, `Block`, `Try`/`catch`/`finally`, `throw`.
9. **Arrays.** `Array`/`LegacyArray` literals, `ArrayAccess`/`ArrayAppend`
   (auto-vivify on LHS), `Foreach`, `list()`/`[]` destructuring, `isset`/`empty`/
   `unset`, indirect/nested variables (read side; refuse `$$x =` write).

(Phases 5–7 — user functions, references, OOP — follow the ast-surface.md §14
phase map and are out of scope for the scaffold gate.)

---

## 6. Pitfalls / do-not-regret list

- **Strings are bytes, not UTF-8.** `PhpStr` = `Rc<BString>`; `strlen` and byte
  offsets count bytes (`"héllo"` is 6). Never route PHP strings through
  `String`/`str`/`char`.
- **Int overflow promotes to float, never wraps.** `PHP_INT_MAX + 1` →
  `float(9.223372036854776E+18)`. Use `checked_*` arithmetic and fall back to
  `f64` on overflow — in `+ - * ** ` and in the `9223372036854775808` literal.
- **Two float precisions.** `echo`/`print`/`.`/`(string)`/interpolation use
  **precision 14** (`0.1+0.2` → `0.3`, sci form `1.0E+20`). `var_dump`/
  `var_export`/`json_encode` use **serialize_precision -1** (shortest
  round-trip: `float(0.30000000000000004)`). Two separate formatters; do not
  share one.
- **Array key normalization is exact.** `(string)(int)s === s` canonical-int-
  string rule; `"08"`/`"+1"`/`" 1"`/`"-0"` stay strings; `true/false→1/0`,
  `null→""`, `float→trunc-toward-zero`. `next_int` is a high-water mark that
  `unset` never lowers; `i64::MAX` append disabled. Get this wrong and every
  array test drifts.
- **COW discipline.** Mutate `Value::Str`/`Value::Array` **only via `make_mut`**
  through the lvalue being written — never clone the inner `Rc` out and mutate
  it. `$b=$a; $b[0]=1;` must not touch `$a`. Arrays are **value** types
  (`Rc<ArrayData>`, no `RefCell`); only `Ref` and objects alias (`RefCell`).
- **PHP 8 loose comparison.** Number-vs-non-numeric-string casts the *number to
  string* (`0 == "a"` is **false**, not PHP 7's true). Two numeric strings
  compare numerically; else byte-wise.
- **Single-threaded `Rc`.** Engine is `!Send`/`!Sync`; use `Rc`/`RefCell`, never
  `Arc`/`Mutex`. Reference cycles leak for now (cycle collector deferred).
- **Keep arenas alive.** One `LocalArena` per file, owned by `ParsedFile`, alive
  for the whole walk. No `&'arena` escapes into `Value`/`Interp`/`Scope`/builtins.
- **Refuse the dangerous set loudly, from day one.** Structurally:
  `Construct::Eval`, `CompositeString::ShellExecute` (backticks),
  `UnaryPrefixOperator::UnsetCast`, and `$$x =` writes. By registry:
  `extract`/`compact`/`create_function`/`assert(string)`/`exec`-family. All →
  `Thrown::Unsupported` rendered with the `RHP_UNSUPPORTED` marker, distinct
  from PHP's own "undefined function" error.
- **`#[non_exhaustive]` enums.** `Statement` (32) and `Expression` (33) both need
  a `_ =>` arm that errors clearly on an unhandled/`Error` node.
- **`DirectVariable.name` includes the leading `$`** — strip it before keying the
  variable table.
