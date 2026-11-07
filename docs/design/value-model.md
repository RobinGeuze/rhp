# rhp runtime value model

Status: design, phase 1 (tree-walking interpreter). Target semantics: PHP 8.5
(ground truth: the reference `php`, see `crates/rhp-builtins/src/version.rs`; probed on 8.5.8). This document fixes the concrete Rust types
for the runtime `Value` (the zval analogue) and its sub-structures.

All behavior claims below were verified against `php` 8.5.8; the probed cases are
reproduced inline so they can be turned into tests.

## 0. Guiding decisions (TL;DR)

- **Single-threaded.** The whole engine is `!Send`/`!Sync`. We use `Rc`/`RefCell`,
  not `Arc`/`Mutex`. This matches PHP's share-nothing request model and the Zend
  engine's own non-atomic refcounting. Reference cycles (e.g. `$a->x = $a`)
  are found by php's trial-deletion cycle collector over `Rc::strong_count`
  (gc.md); it did not change these types.
- **Values own all their data.** No variant of `Value` (or anything reachable
  from it) carries the `'arena` lifetime. The arena holds the *AST*; the runtime
  holds *owned* copies. See §7.
- **Two different sharing disciplines, kept strictly separate:**
  - *value types with copy-on-write* (`string`, `array`) use `Rc<T>` +
    `Rc::make_mut` — cheap clone, deep-on-write, **value semantics**.
  - *handle/reference types* (`object`, PHP `&` references) use
    `Rc<RefCell<T>>` — shared mutable cell, **aliasing semantics**.
  Conflating these (e.g. putting arrays behind `RefCell`) would break PHP's
  value semantics, so we don't.

## 1. The `Value` enum (zval analogue)

```rust
use std::rc::Rc;
use std::cell::RefCell;

/// The runtime value. Analogue of Zend's `zval`. Owned, cheaply cloneable,
/// `'static` (no arena borrow). ~16 bytes (one 8-byte payload + tag).
#[derive(Clone)]
pub enum Value {
    /// PHP `null`.
    Null,
    /// PHP `bool`.
    Bool(bool),
    /// PHP `int`: always 64-bit signed, matching PHP on 64-bit platforms.
    Int(i64),
    /// PHP `float`: IEEE-754 double. Plain `f64` (NOT `OrderedFloat`) — floats
    /// are never hashed or used as map keys at runtime (they truncate to int
    /// keys, see §3), so we keep native `==`/`<` with correct NaN behavior.
    Float(f64),
    /// PHP `string`: binary-safe byte string, COW. See §2.
    Str(PhpStr),
    /// PHP `array`: ordered int|string -> value map, COW value semantics. See §3.
    Array(PhpArray),
    /// PHP `object`: shared handle (reference-handle semantics). See §5.
    Object(ObjectHandle),
    /// A PHP reference (`&`): a shared mutable slot. See §4.
    /// `Ref` is NOT a PHP type — it is an internal marker that a slot is aliased.
    Ref(PhpRef),
}
```

Representation rationale, variant by variant:

- `Null`, `Bool(bool)`, `Int(i64)` — trivial unboxed scalars. PHP `int` is
  64-bit; the parser yields integer literals as `Option<u64>` (decimal digits),
  and we fold to `i64`, falling back to `Float` on overflow exactly as PHP does
  (`9223372036854775808` parses to a float literal in PHP).
- `Float(f64)` — unboxed. We deliberately do **not** reuse mago's
  `OrderedFloat<f64>` here: ordered-float exists to make floats `Hash`+`Ord` for
  AST dedup, but runtime floats need IEEE `==` (where `NAN != NAN`) and are never
  keys. `ordered-float` stays available if a future need (e.g. a float-keyed
  interning cache) appears, but the core value model does not use it.
- `Str`, `Array`, `Object`, `Ref` — each is a single pointer (`Rc<..>`), so the
  enum stays small (16 bytes) and `Clone` is always O(1) pointer work plus a
  non-atomic refcount bump. This keeps the hot path (copying a value into a
  variable, pushing an argument) cheap, which is the dominant operation in a
  tree-walker.

> **Today** the enum also has `Closure`, `Resource` and `Undef`. `Undef` is
> php's IS_UNDEF: the state of a variable slot (a CV, or a symbol-table cell
> such as a global's) that was never assigned or was unset. It is never a
> value: `clone()` of `Undef` gives `Null`, so a copy read out of an
> undefined slot is null and the marker cannot leak into arrays, arguments or
> returns. `copy_slot()` keeps it (an arrow function's implicit capture, an
> include's shared cell). See bytecode.md, "Undefined variables".

`Value: Clone` is the engine's workhorse: assigning `$b = $a` is
`slot_b = value_a.clone()`. For scalars that is a bit-copy; for `Str`/`Array`
it is an `Rc` bump that shares the buffer until the first write (COW); for
`Object`/`Ref` it shares the handle (aliasing). This is precisely PHP's
copy-on-assign model.

## 2. `PhpStr` — binary-safe, COW byte string

PHP strings are **byte strings, not UTF-8**: they can hold NULs and arbitrary
bytes, `strlen` counts bytes, offsets index bytes. So `String`/`str` are wrong.

```rust
use bstr::BString;

/// Owned, binary-safe, copy-on-write PHP string.
/// Clone is O(1) (refcount bump). Mutation is copy-on-write via `make_mut`.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PhpStr(Rc<BString>);

impl PhpStr {
    pub fn from_bytes(b: impl Into<Vec<u8>>) -> Self {
        PhpStr(Rc::new(BString::new(b.into())))
    }
    pub fn as_bytes(&self) -> &[u8] { self.0.as_slice() }
    pub fn len(&self) -> usize { self.0.len() }

    /// COW mutable access: clones the buffer iff it is currently shared.
    pub fn make_mut(&mut self) -> &mut BString { Rc::make_mut(&mut self.0) }
}
```

**Why `Rc<BString>`:**

- **Cheap clone.** `Rc` gives O(1) clone, mirroring Zend's refcounted
  `zend_string`. Strings are copied constantly (every variable read into a temp,
  every argument), so O(len) clone would dominate.
- **Mutable on write.** `.=` (append), `$s[$i] = 'x'` (byte assignment),
  `substr_replace`, etc. need in-place mutation. `BString: Clone`, so
  `Rc::make_mut` clones the buffer *only* when the `Rc` is shared (refcount > 1)
  and otherwise mutates in place — exact COW, and it keeps us from aliasing two
  PHP string values through one buffer.
- **`bstr` ergonomics.** `BString`/`BStr` give us a full byte-string API (`find`,
  `split`, case ops) and a *lossy* `Display`/`Debug` for diagnostics, without
  pretending the bytes are UTF-8.

**Rejected alternatives:**

- `Rc<[u8]>` — single indirection and immutable, but `[u8]: !Clone` means
  `Rc::make_mut` is unavailable, so there is no clean COW mutation path; every
  append would hand-roll a realloc+copy. Rejected.
- `bstr::BString` (owned, no `Rc`) — clone is O(len); loses the cheap-clone
  property that dominates the hot path. Rejected as the *value* representation
  (it is the right thing *inside* the `Rc`).
- Small-string optimization (inline ≤ N bytes, e.g. a hand-rolled SSO enum over
  `smallvec::SmallVec<[u8; 23]>`) — attractive because PHP creates many tiny
  strings (keys, identifiers), **but** inline storage makes clone O(len) again,
  defeating cheap clone, and no SSO crate in our tree is binary-safe
  (`compact_str` is UTF-8). **Deferred.** `PhpStr` is a newtype wrapping one
  `Rc`, so we can later turn it into `enum { Inline([u8; N], u8), Shared(Rc<BString>) }`
  or add an interned-symbol variant for class/function names without touching
  call sites.

Hashing/equality are over the raw bytes (what PHP array keys need), so
`#[derive(Hash, Eq)]` on the byte content is correct.

## 3. `PhpArray` — ordered map, COW value semantics

PHP arrays are **insertion-ordered** maps keyed by `int` *or* `string`, with a
running "next free integer index". Value semantics with copy-on-write.

```rust
use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;

/// PHP array key: canonicalized to int-or-string (see normalization below).
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum ArrayKey {
    Int(i64),
    Str(PhpStr),
}

/// The array buffer. Insertion order preserved by IndexMap.
#[derive(Clone)]
pub struct ArrayData {
    /// Ordered entries. FxBuildHasher: fast, non-DoS-resistant hashing — fine
    /// for a single request (PHP itself is not collision-hardened by default).
    pub entries: IndexMap<ArrayKey, Value, FxBuildHasher>,
    /// php's nNextFreeElement: the key `$a[] = ...` uses. `i64::MIN` until
    /// an integer key is inserted (a fresh array appends at 0).
    next_free: i64,
    /// Internal pointer (`current`/`next`/...); `entries.len()` is past the end.
    pub pos: usize,
    /// Positions of array_walk's HashTable iterators, by id.
    iters: Option<Box<Vec<(u32, usize)>>>,
}

/// A PHP array value. Clone is O(1) (Rc bump); mutation is COW via `make_mut`.
#[derive(Clone)]
pub struct PhpArray(Rc<ArrayData>);

impl PhpArray {
    pub fn get(&self, k: &ArrayKey) -> Option<&Value> { self.0.entries.get(k) }

    /// COW mutable access: clones the whole buffer iff shared, then mutate.
    pub fn make_mut(&mut self) -> &mut ArrayData { Rc::make_mut(&mut self.0) }
}
```

### 3a. Copy-on-write

`$b = $a` clones the `Rc` (shared buffer). Any mutation of `$b` goes through
`PhpArray::make_mut` → `Rc::make_mut`, which clones `ArrayData` **iff** the
refcount is > 1. The clone is *shallow in the Rust sense*: `IndexMap: Clone`
clones each `Value`, and every `Value` clone is itself cheap (`Rc` bump / scalar
copy). This yields exactly PHP's COW semantics, including the subtle rules:

- Nested arrays are themselves `Rc`-shared after the shallow clone and only
  deep-copy on their own first write — PHP's recursive COW.
- A **reference element** (`Value::Ref`) is shared, not copied, by the shallow
  clone — matching PHP, where `&` elements survive an array copy as live
  aliases.

**Why `Rc<ArrayData>` and NOT `Rc<RefCell<ArrayData>>`:** arrays are *value*
types. Mutation always happens through an lvalue (a variable slot, an array
element, a property) where the evaluator holds `&mut Value`, so exclusive access
is available statically and `Rc::make_mut` is the correct COW primitive. Wrapping
the array in `RefCell` would (a) add needless runtime borrow checks and (b) tempt
aliasing that would make `$b = $a; $b[0]=1;` mutate `$a` — i.e. silently turn
arrays into reference types, which is wrong. `RefCell` is reserved for genuine
aliasing (`Value::Ref`, objects). The required COW invariant is therefore:

> **COW invariant:** mutate a `Value::Str`/`Value::Array` only via its
> `make_mut` (never by cloning the inner `Rc` out and mutating it), so a write
> can only ever be observed through the lvalue being written.

### 3b. Key normalization (verified against php 8.5.8)

A key of any type is canonicalized to `ArrayKey` by `ArrayKey::from_value`:

| Source value            | Result key        | Note |
|-------------------------|-------------------|------|
| `int n`                 | `Int(n)`          | |
| `bool true` / `false`   | `Int(1)` / `Int(0)` | |
| `null`                  | `Str("")`         | emits deprecation notice in 8.5 |
| `float f`               | `Int(trunc(f))`   | truncate toward zero; deprecation notice if `f` had a fractional part |
| `string s`              | canonical-int rule below |

**Canonical-integer-string rule** (the rule for `string -> Int`): a string key
is converted to an integer key **iff it is the canonical decimal form of an
`i64`** — equivalently, iff `(string)(int)s === s`. Concretely, `s` must match
`^-?(0|[1-9][0-9]*)$`, be in `i64` range, and `"-0"` does not qualify (its
canonical form is `"0"`). Otherwise the string is kept verbatim as `Str`.

Probed (`php` 8.5.8), each `=> Int` or `=> Str` as marked:

```
"0"  => Int 0        "8"  => Int 8        "-5" => Int -5
"08" => Str          "00" => Str          "01" => Str        // leading zero
"+1" => Str          " 1" => Str          "1.5"=> Str        // sign/space/dot
"9223372036854775807" => Int (i64::MAX)
"9223372036854775808" => Str              // overflows i64
true => Int 1   false => Int 0   null => Str ""   2.9 => Int 2 (toward zero)
```

`trunc(f)` for float keys follows PHP's `zend_dval_to_lval`: toward-zero
truncation, with `NAN`/±`INF`/out-of-range folding to `0` (we match PHP's
observed result rather than Rust's `as i64` saturation; a shared
`float_to_int(f64) -> i64` helper encodes this and is reused by `(int)` casts).

### 3c. Next-free-index counter

`next_free` follows php's nNextFreeElement. It starts at `i64::MIN` (a
fresh array appends at 0). Inserting integer key `k` (append or explicit)
sets `next_free = k + 1` when `k >= next_free`, saturating at `i64::MAX`.
`unset` never lowers it; `array_pop` lowers it by one when it removes the
key `next_free - 1`, and `array_shift`/the sorts that renumber set it to
the new count. Verified (php 8.5):

```
[10=>x][]      -> keys 10,11
[-5=>a][]      -> keys -5,-4          // first int key sets next even when negative
[-5=>a,-3=>c][] -> keys -5,-3,-2
["z"][]; [-5]=m; []  -> keys 0,1,-5,2 // -5 (<next) doesn't move next
[0,1,2]; unset(2); [] -> keys 0,1,3   // high-water survives unset
[-1=>0]; array_pop; []  -> key -1
```

An append when `next_free` is `i64::MAX` and that key is taken fails
(`PhpArray::try_push` / `append_mut` return false / `None`): `$a[] = v`,
array literals, unpacking and `array_push` throw `Error: Cannot add
element to the array as the next element is already occupied`.

### 3d. Internal pointer and iterators

php's internal pointer and HashTable iterators are bucket positions; a
deleted bucket leaves a hole, so a position keeps pointing at the same
element. IndexMap compacts on removal instead, so `PhpArray::remove` (and
`ArrayData::remove_index`) move `pos` and every registered iterator back
by one when the removed entry was before them. A removed current element
leaves the pointer on the next one, and the past-the-end position
(`entries.len()`) picks up a later append, as php's `nNumUsed` does. A
copy-on-write separation restarts a past-the-end pointer at 0, as
`zend_array_dup` does.
`array_walk` registers an iterator (`iter_add`/`iter_pos`/`iter_del`); it
is copied along with the array, so the walk survives a copy-on-write
separation, and a different array (the variable reassigned) is walked
from its internal pointer. The sorts keep iterator positions, and
`array_splice` moves them the way `php_splice` does (`splice_iterators`).

## 4. Reference semantics (PHP `&`)

A PHP reference makes two names denote the *same* storage. We model a reference
as a shared mutable cell:

```rust
/// A PHP reference: a shared, mutable value slot.
pub type PhpRef = Rc<RefCell<Value>>;
```

A variable/array/property **slot** normally holds a `Value` *directly*
(not a ref). A slot becomes a reference only when `&` is applied; this is called
**promoting** the slot:

- **`$b = &$a`:** promote `$a`'s slot — if it is not already `Value::Ref(rc)`,
  take the current `Value`, wrap it as `rc = Rc::new(RefCell::new(v))`, and store
  `Value::Ref(rc.clone())` back into `$a`'s slot; store `Value::Ref(rc)` into
  `$b`'s slot. Now both slots hold the *same* `Rc<RefCell<Value>>`.
- **Reading a variable:** if the slot is `Value::Ref(rc)`, the effective value is
  `rc.borrow().clone()` (deref); otherwise the slot value itself. A small
  `deref_once`/`deref_fully` helper centralizes this.
- **Writing through a name:** if the slot is `Value::Ref(rc)`, write into
  `*rc.borrow_mut()`; otherwise overwrite the slot. (Writing a *new* `&`
  rebinds the slot to a different `rc`.)
- **By-ref parameter `function f(&$p)`:** the caller promotes the argument's
  slot and binds the callee's `$p` slot to the same `Value::Ref(rc)`. Writes to
  `$p` are visible at the call site. A non-variable follows php's SEND_* rules
  (below).
- **`foreach ($arr as &$v)`:** each iteration promotes the current **array
  element's** slot (the element becomes `Value::Ref(rc)` *inside the array*) and
  binds `$v` to the same `rc`. Writes to `$v` alias the element. Because the
  element now holds a `Value::Ref`, the array's COW shallow-clone shares that
  ref — so the reference stays live even if the array is copied, exactly as PHP
  (the well-known "`&` left in an array after foreach" footgun is reproduced
  faithfully). The loop keeps a HashTable iterator on the array (see the
  foreach section of bytecode.md), so it sees elements added or removed.

**Non-variables where php wants a reference.** As in php, the compiler
classifies the expression (zend_is_variable / zend_is_call) and the VM
decides when it runs:

- A by-reference function always returns a `Value::Ref` (`RetRef`); a
  by-value one never does. So "the call returned a reference" is a
  runtime test on the result register.
- `$x =& f()` (`RefFuncResult`): a value notices "Only variables should be
  assigned by reference" and is assigned (through `$x`'s current
  reference). `return <non-variable>` in a by-reference function (and its
  implicit `return null`) notices "Only variable references should be
  returned by reference" and returns a new reference. The return type of a
  by-reference function is checked on the referenced value, which keeps the
  coerced value, and again after `finally` blocks ran.
- By-reference arguments: a call result, `new` or `$a =& $b` (php's VAR
  results) passes a reference it holds or notices "Only variables should be
  passed by reference" (`SendNoRef`); any other temporary throws "f():
  Argument #N ($p) could not be passed by reference". For a callee known
  only at run time the compiler records the argument's kind in
  `FunctionProto::send_kinds` (call pc, index, `SEND_VAL`/`SEND_VAR`) and
  `bind_frame` checks it when a by-reference parameter meets a value, so
  ordinary calls pay nothing.
- `[&$a, [&$b]] = $src`: the source is fetched for writing and made a
  reference, then each element is fetched as one (`FetchListW`, php's
  FETCH_LIST_W); a by-value call result notices "Attempting to set
  reference to non referenceable value". `foreach ($a as [&$x])` iterates
  by reference.
- `f(...$arr)` for by-reference parameters makes those elements
  references inside the (separated) array (`ArgSpreadRef`); a Traversable's
  warn and go by value.

**Normal value vs reference is distinguished structurally by the `Value::Ref`
variant.** There is no separate "is_ref" flag: an aliased slot *is* a
`Value::Ref`, an unaliased slot is any other variant. This mirrors Zend's
`IS_REFERENCE` zval type. `RefCell`'s borrow tracking catches the rare
aliasing-violation bug at runtime (e.g. simultaneous `&mut` into one ref during
evaluation) instead of letting it be silent UB.

**Reference counts are observable.** `var_dump` marks a slot `&` when its
reference has refcount > 1 (`Rc::strong_count`), and `debug_zval_dump` prints
the counts. So copies follow php: `ArrayData::clone` (zend_array_dup) and the
builtins that build arrays from other arrays' elements (`array_values`,
`array_merge`, `array_slice`, …, array spread) copy a reference nothing else
holds as its value (`ops::copy_elem`), and a temporary reference (the one a
by-reference `foreach` over `$a['k']` holds) is released when php frees it.
Where rhp keeps a second holder php does not have (a global's cell: the
globals table and main's CV; an enum case: its constant and the case list),
`Host::shadow_holders` reports it and the dumpers leave it out.

**Typed references** (php's ZEND_REF_TYPE_SOURCES). A reference bound to
a typed property remembers it as a type source; writes through it are
checked and coerced against every source ("Cannot assign string to
reference held by property C::$p of type int", the inconsistent-coercion
error for two sources), and binding a reference to a typed property checks
it first. `PhpRef` stays a plain `Rc<RefCell<Value>>`: the sources live in
the VM's side table (`rhp-vm` refsrc.rs) keyed by the cell's address, with a
`Weak` to the cell so the address is not reused while the entry lives. A
source names the object weakly (or the static property); one whose object
is gone or whose slot no longer holds the cell is skipped, so unset, rebind
and object free need no hook (php deletes the source; the explicit sites
here still delete it, keeping php's order). Untyped references pay one
`is_empty` branch while no typed reference exists, one hash lookup after.
Cursor writes find a typed reference inside `assign_path`, which hands the
write back to the VM (`TypedStop`). Builtins write by-reference arguments
through `Host::assign_ref` (settype, the preg functions; extract assigns
through the scope); the other by-ref writers do not check yet. Not done:
by-ref foreach over an ArrayIterator backed by an object (php's
spl_array_it_get_current_data adds the source): rhp's ArrayIterator copies
the object's properties; unserialize() does not add sources for references
to typed properties (php's var_unserializer does).

## 5. Object model sketch (enough for phase 7)

Objects are **handle types**: a variable holds a handle, and `$b = $a` copies the
*handle*, so both names see the same object (mutations shared; identity `===`
holds). This is reference-handle semantics, distinct from array value semantics.

```rust
/// Interned handle to a class definition (class table lives in the engine, not
/// in the value). An index/id keeps `Value` cheap and `'static`.
pub type ClassId = u32;

pub struct ObjectData {
    /// Which class this is an instance of.
    pub class: ClassId,
    /// The object handle (spl_object_id, `#N`): php's object store, reused
    /// once the object is freed (rhp_values::handles).
    pub id: u32,
    /// Declared properties in the class's slots, then dynamic ones in
    /// insertion order (`PropTable`, oop.md §1.1). Values may themselves be
    /// `Value::Ref` for `&`-bound properties.
    properties: PropTable,
    /// Native payload, lazy object state, weak flag, iterators (boxed).
    extra: Option<Box<ObjectExtra>>,
}

/// Shared object handle. Clone shares (handle semantics). Interior mutability
/// via RefCell because many live handles can mutate the same object.
pub type ObjectHandle = Rc<RefCell<ObjectData>>;
```

- **Assignment / passing** copies the `Rc` → shared handle. No COW.
- **`clone $o`** performs a shallow property copy: new `id`, new `Rc`, cloned
  property table (then `__clone` runs). This is the one place objects "copy".
- **Identity `===`** is `Rc::ptr_eq` (equivalently equal `id`); `==` compares
  class + property values.
- **`&` properties** are represented by `Value::Ref` property values, so the
  §4 machinery is reused unchanged.

`ClassId` (not an `Rc<ClassEntry>`) keeps the object independent of class-table
lifetime and keeps `Value` pointer-sized. Cycles (`$o->self = $o`) leak under
`Rc` until the deferred cycle collector lands.

## 6. Relationship to the arena

mago-syntax is arena-allocated: `parse_file_content(&arena, id, src)` returns
`&'arena Program<'arena>`, and literals arrive **borrowed** — e.g.
`LiteralString { value: Option<&'arena [u8]> }`, `LiteralInteger { value:
Option<u64> }`, `LiteralFloat { value: OrderedFloat<f64> }`. The evaluator reads
these borrows but, when it produces a runtime `Value`, it **copies** the bytes
into an owned `PhpStr` (`PhpStr::from_bytes(lit.value)`), and folds integer/float
literals into owned `Value::Int`/`Value::Float`.

Therefore:

- **No `Value` variant, nor anything transitively reachable from a `Value`
  (`PhpStr`, `ArrayData`, `ObjectData`, `PhpRef`), carries a `'arena` lifetime.**
  Every one is effectively `'static`.
- A `Value` stored in a variable, array, or property **outlives the statement**
  that produced it and does not pin the arena. The arena can be dropped after a
  script finishes (or per-compilation-unit) without dangling any live value.
- The arena's job is the *program* (AST + spans + interned identifiers consumed
  during evaluation); the runtime heap's job is *data* (owned `Rc` graphs). The
  two never share allocations.

This separation is the reason the value types above are built on `Rc`/owned
buffers rather than borrowing: runtime data must outlive any `&'arena` borrow.

## 7. Dependency notes

- `indexmap` 2.14 — ordered maps for arrays and object properties.
- `bstr` 1.13 — binary-safe string buffer (`BString`/`BStr`) inside `PhpStr`.
- `rustc-hash` (`FxBuildHasher`) — fast non-cryptographic hashing for the
  per-request maps. Present transitively in the lockfile; add it as a direct dep
  of the runtime crate. (If DoS-resistant hashing is ever wanted it can be
  swapped per-map without type changes.)
- `hashbrown` — available; `IndexMap` already builds on it, so we don't use it
  directly unless we need a raw table.
- `ordered-float` — used by mago's AST (`LiteralFloat`); **not** used in the
  value model (runtime `Float` is plain `f64`; floats are never keys). Kept in
  reserve.
- `smallvec` — reserved for a future `PhpStr`/argument-vector SSO; not in the
  core types yet.
