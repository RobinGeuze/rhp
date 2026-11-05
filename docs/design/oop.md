# rhp OOP — authoritative build spec (phase 7, VM-only)

Status: **build spec. Implement from this.** Target semantics: PHP 8.5 (ground
truth: the reference `php`, see `crates/rhp-builtins/src/version.rs`; every behavior below was probed
against it, originally 8.5.8). This is
the single, non-contradictory synthesis of the four research docs
([`oop-ast.md`](./history/oop-ast.md), [`oop-model.md`](./oop-model.md),
[`oop-isa.md`](./history/oop-isa.md), [`oop-prior-art.md`](./history/oop-prior-art.md)) and the
continuation of [`bytecode.md`](./bytecode.md) §2.4 ("phase 7: `New`,
`FetchProp`, `CallMethod`, `CallStatic`, `InstanceOf`, `Clone`, late static
binding — same encoding and calling convention; omitted there"). Where the
research docs disagreed, this doc **picks one** and says so; where they agreed,
this doc is the terse union. Nothing in `bytecode.md`/`value-model.md` changes;
this only *adds* `Instr` variants, side tables, and `Vm`/`Frame`/`FunctionProto`
fields.

**The only parity net is `php ⟷ vm`** (`bytecode.md` §6). That raises the bar on
first-time correctness — hence the inline probes, which must become golden
fixtures.

## Locked decisions (resolving the cross-doc conflicts)

1. **Flatten inheritance at link time**, never walk the chain on the hot path
   (all four docs agree). One hash probe per method lookup, one index per
   property. (§1, §6)
2. **`$this` lives in a CV, not a `Frame` field.** `FunctionProto.this_cv:
   Option<Reg>` names the CV; instance methods reserve CV 0. (Adopts
   oop-model/oop-isa over the prior-art sketch of `Frame.this:
   Option<ObjectHandle>` — keeps `$this` an ordinary variable with no new frame
   field. §4)
3. **No `AssignProp` opcode.** Property *writes* flow through the existing lvalue
   cursor (`LvalProp` + `AssignLval`), so `$o->p = v`, `$o->p[$i] += 1`,
   `$o->p++`, and nested `$o->a->b[$i]` all reuse one path. (**Adopts
   oop-isa.md §2.2; overrides the `AssignProp` sketch in oop-model.md §9.**) §3
4. **Properties stored name-keyed in an `IndexMap`** (already fixed in
   `value-model.md` §5), with the inline-cache `payload` reserved to later become
   a dense slot index — an internal, ISA-preserving change. §1, §5
5. **Late static binding via two `Option<ClassId>` on the `Frame`:**
   `defining_class` (→ `self::`/`parent::`/`__CLASS__`) and `called_class` (→
   `static::`/`new static`/`static::class`). §4
6. **Visibility is parsed, stored, and enforced** with a caller-class check
   (`caller == frame.defining_class`). Private-name shadowing uses php's mangled storage
   keys (see oop-model.md §4). Documented simplification: typed-uninitialized
   properties materialize as `Null`. §5

---

## 1. Runtime object + class model

### 1.1 `ObjectData` — confirmed, unchanged

Already built in `rhp-values` (`value-model.md` §5). **No change for cut 1.**

```rust
// rhp-values
pub struct ObjectData {
    pub class: ClassId,                                  // u32 index into Vm.classes
    pub id: u32,                                         // spl_object_id (monotonic)
    pub properties: IndexMap<PhpStr, Value, FxBuildHasher>, // declaration+insertion order
}
pub type ClassId = u32;
pub type ObjectHandle = Rc<RefCell<ObjectData>>;         // Value::Object(ObjectHandle)
```

Handle semantics (invariant): assignment shares the `Rc`; `clone` makes a new
`Rc` + fresh `id` + shallow-copied map; `===` is `Rc::ptr_eq`. The class table is
**in the `Vm`**, never in the `Value`. (The prior-art `slots: Vec<Value>` +
lazy `dynamic` hybrid is the documented *later* optimization; cut 1 keeps the
single `IndexMap`.)

### 1.2 Leaf types (`rhp-values` — small, `Copy` where possible)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum ClassKind { Class, Interface, Trait, Enum }
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Visibility { Public, Protected, Private }
#[derive(Clone, Copy, Default, Debug)]
pub struct ClassFlags { pub is_abstract: bool, pub is_final: bool, pub is_readonly: bool }
```

### 1.3 The linked class table — `ClassEntry` (`rhp-vm`, built at install)

One per declared class/interface/trait/enum, indexed by `ClassId` in
`Vm.classes`. **All tables flattened** (parent merged first, own members
override). This is the canonical shape — it unifies oop-model's `PropEntry`/
visibility metadata with oop-isa's `prop_template`/`ancestors`/`static_props`
cells:

```rust
// rhp-vm
pub struct ClassEntry {
    pub id: ClassId,
    pub name: PhpStr,                 // original case (messages / get_class / ::class)
    pub kind: ClassKind,
    pub flags: ClassFlags,
    pub parent: Option<ClassId>,      // `extends` (single; None for root/interface/trait)

    /// self + all transitive parents + all transitive interfaces. The instanceof
    /// / catch membership set (O(1) test, never a walk).
    pub ancestors: Vec<ClassId>,

    /// FLATTENED vtable: lowercased method name -> entry (names case-insensitive).
    pub methods: HashMap<Vec<u8>, ResolvedMethod>,

    /// FLATTENED instance properties, in PHP declaration order (parent props keep
    /// their positions; child's NEW props appended; an override replaces the entry
    /// IN PLACE). Carries both the per-instance default and visibility metadata;
    /// the IndexMap gives order (for var_dump/(array)/foreach) and O(1) get_index
    /// for the IC slot payload. Names are case-sensitive.
    pub props: IndexMap<PhpStr, PropEntry>,

    /// FLATTENED class constants, lazy + memoized (initializers may reference
    /// self::/parent::/other consts). Case-sensitive.
    pub constants: HashMap<Vec<u8>, ClassConst>,

    /// FLATTENED static-prop slots. An inherited slot SHARES the parent's Rc cell;
    /// a redeclared one gets a fresh cell. Case-sensitive.
    pub static_props: IndexMap<Vec<u8>, StaticProp>,

    /// __construct resolved in the flattened `methods`, cached at link. None => no ctor.
    pub constructor: Option<Rc<FunctionProto>>,
}

pub struct ResolvedMethod {
    pub proto: Rc<FunctionProto>,
    pub defining_class: ClassId,   // where the BODY is declared (self::/parent::/private);
                                   // for a trait method, the USING class.
    pub visibility: Visibility,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_final: bool,
}

pub struct PropEntry {
    pub default: PropDefault,
    pub visibility: Visibility,
    pub is_readonly: bool,
    pub declaring_class: ClassId,  // for visibility + (future) private shadowing
}

/// Per-instance initial value of a declared property.
pub enum PropDefault {
    Value(Value),            // constant default, folded at compile time (`= 1`, `= [1,2]`)
    Init(Rc<FunctionProto>), // non-constant initializer (`= self::F + 1`, `= 1 << 3`): a
                             // zero-param thunk run per-instance at `new`, bound to this class
    Null,                    // declared without default (typed-uninit also lands here, cut 1)
}

pub struct StaticProp {
    pub cell: Rc<RefCell<Value>>,  // inherited statics CLONE this Rc (alias); redeclare = fresh Rc
    pub visibility: Visibility,
    pub declaring_class: ClassId,
    pub is_readonly: bool,
    pub init: Option<Rc<FunctionProto>>, // lazy non-const static init, run once on first use
}

pub struct ClassConst {
    pub visibility: Visibility,
    pub declaring_class: ClassId,
    pub state: RefCell<ConstState>,
}
pub enum ConstState {
    Resolved(Value),
    Thunk(Rc<FunctionProto>), // first read evaluates + memoizes
    Resolving,                // cycle-detection sentinel (self-referential const => fatal Error)
}
```

### 1.4 `Vm` growth

```rust
pub struct Vm {
    // ... existing: out, stack, functions, fn_names, constants, max_depth, cursor_* ...
    classes: Vec<ClassEntry>,
    class_ids: HashMap<Vec<u8>, ClassId>,   // lowercased name -> id (names case-insensitive)
}
```

Object handles come from `rhp_values::handles`, php's object store: freed
handles are reused most recently freed first (see oop-model.md, "Fresh id").

### 1.5 Inheritance strategy: **flatten at registration** (the one choice)

Rationale (all four docs concur, isomorphic to Zend `zend_do_inheritance`):

- **Dispatch is O(1) and IC-cacheable.** `$o->m()` is
  `classes[o.class].methods[lc(m)]` — one probe, guarded by a single `ClassId`
  compare. A walk-the-chain model defeats the single-`ClassId` IC guard.
- **PHP is single-implementation-inheritance** → no C3/MRO. The only linear chain
  is the `parent` spine; interfaces add type identity only; traits are flattened
  into the using class at link.
- **Cost paid once** per class at link (O(total members)), vs O(depth) on every
  hot access. Downside (memory + a parent-before-child link ordering constraint)
  is handled by topological linking (§6).

Must-reproduce semantics (→ fixtures):

```php
class A { public function hi(){return "A::hi";} public function go(){return $this->hi();} }
class B extends A { public function hi(){return "B::hi + ".parent::hi();} }
(new B)->go();   // "B::hi + A::hi"  (virtual dispatch on B; parent:: is static to A)

class A2 { public $x = 1; public $arr = [1,2]; protected $p = "a"; }
class B2 extends A2 { public $y = 2; public $x = 10; }
// var_dump(new B2) property order: x(=10), arr, p, y
//   inherited x/arr/p keep A2's positions; x's VALUE overridden in place; y appended.
$o = new B2; $o->arr[] = 3;  (new B2)->arr;  // [1,2]  — per-instance COW, not shared
```

---

## 2. `FunctionProto` / `Program` growth + hoisting & registration

### 2.1 `FunctionProto` — two new fields, **no `ClassId`**

```rust
// rhp-bytecode
pub struct FunctionProto {
    // ... all existing fields unchanged ...
    pub this_cv: Option<Reg>,  // CV holding $this for an instance method; None otherwise
    pub is_static: bool,       // true for `static function` (no-$this binding path)
}
```

The proto deliberately stores no defining `ClassId`: protos are compiled before
linking assigns ids and are `Rc`-shared. The defining class travels on
`ResolvedMethod.defining_class` → `Frame.defining_class`; LSB uses
`Frame.called_class`. So `self::`/`parent::`/`static::` resolve from the frame
and **no proto needs a post-link fixup**. `__METHOD__`/`__FUNCTION__` read
`proto.name`; `__CLASS__` reads `frame.defining_class`.

### 2.2 `Program.classes` — the un-linked compiled form

Replace the placeholder `pub classes: Vec<PhpStr>` (currently line 173; the
`Vec<ClassEntry>` comment at line 255 is the intent) with:

```rust
// rhp-bytecode
pub struct Program {
    pub main: Rc<FunctionProto>,
    pub functions: Vec<Rc<FunctionProto>>,
    pub classes: Vec<ClassProto>,     // compiled, UN-LINKED; linked by the Vm (§6)
}

/// One compiled declaration; parents/interfaces/traits referenced BY NAME
/// (resolved to ClassId at link). Data only — no Rc cells yet.
pub struct ClassProto {
    pub name: PhpStr,
    pub kind: ClassKind,
    pub flags: ClassFlags,
    pub parent: Option<PhpStr>,         // `extends` (single, for classes)
    pub interfaces: Vec<PhpStr>,        // `implements` / interface `extends` (N)
    pub traits: Vec<PhpStr>,            // `use T;` to merge in
    pub methods: Vec<MethodProto>,
    pub props: Vec<PropProto>,
    pub static_props: Vec<StaticPropProto>,
    pub constants: Vec<ConstProto>,
}

pub struct MethodProto {
    pub name: PhpStr, pub proto: Rc<FunctionProto>,
    pub visibility: Visibility, pub is_static: bool, pub is_abstract: bool, pub is_final: bool,
}
pub struct PropProto      { pub name: PhpStr, pub visibility: Visibility, pub is_readonly: bool, pub default: PropDefaultProto }
pub struct StaticPropProto{ pub name: PhpStr, pub visibility: Visibility, pub is_readonly: bool, pub init: ConstInitProto }
pub struct ConstProto     { pub name: PhpStr, pub visibility: Visibility, pub value: ConstInitProto }

pub enum PropDefaultProto { Const(CIdx), Init(Rc<FunctionProto>), None }
pub enum ConstInitProto   { Const(CIdx), Thunk(Rc<FunctionProto>) }
```

A `CIdx` indexes the `consts` of whichever `FunctionProto` owns the thunk (or
`main` for top-level folded literals). Non-constant initializers compile to
zero-param `Rc<FunctionProto>` thunks that run on the VM like any proto — **no
second evaluator**: property/const/static initializers reuse the compiler and VM
wholesale.

### 2.3 Hoisting + registration (handling `extends` ordering / forward refs)

- **Top-level declarations are hoisted.** The compiler's existing function-hoist
  pre-pass is extended to also emit a `ClassProto` per top-level
  `Statement::{Class,Interface,Trait,Enum}` into `Program.classes`, **by name**,
  before top-level code is compiled. Forward references (`class A extends B` where
  `B` is declared later in the file) resolve fine because linking is a separate,
  order-independent pass (§6).
- **Conditional (non-top-level) declarations** — a class inside an `if` — are
  **not** hoisted: the compiler emits a new `DeclareClass { class_idx }` opcode
  (alongside the reserved `DeclareFn`), and the VM links that one `ClassProto` at
  execution point. Its dependencies (parent/interfaces/traits) must already be
  linked by then, exactly as PHP requires.

---

## 3. OOP ISA additions (locked)

All operands are small `Copy` indices/immediates; `Instr` stays `Copy`. All heap
data (names, class protos, caches) stays in side tables.

### 3.1 The class operand `ClassRef` (`rhp-bytecode`, `Copy`, owns no heap)

```rust
#[derive(Clone, Copy, Debug)]
pub enum ClassRef {
    Name(NIdx),     // `Foo` — interned name; runtime resolves to ClassId (IC-cacheable)
    SelfClass,      // `self`   — current frame's DEFINING class
    ParentClass,    // `parent` — parent of the frame's defining class
    StaticClass,    // `static` — current frame's CALLED class (LSB)
    Dynamic(Reg),   // `new $cls()` / `$cls::m()` — runtime string|object in a register
}
```

One VM helper resolves it: `resolve_class(&self, top, ClassRef) -> Result<ClassId,
Thrown>`. For `Dynamic`, a `Value::Str` resolves by name, a `Value::Object`
resolves to its class.

### 3.2 New `Instr` variants

```rust
// instantiation
New { dst: Reg, class: ClassRef, base: Reg, argc: u16, cache: CacheIdx },
// `new C(args)` — alloc into dst FIRST, then run __construct via the normal arg window.

// reads (value context)
FetchProp        { dst: Reg, obj: Reg,   name: NIdx, cache: CacheIdx }, // $o->p
FetchStaticProp  { dst: Reg, class: ClassRef, name: NIdx, cache: CacheIdx }, // C::$p
FetchClassConst  { dst: Reg, class: ClassRef, name: NIdx, cache: CacheIdx }, // C::K / C::class

// calls (push a frame via the normal calling convention)
CallMethod { dst: Reg, obj: Reg,   name: NIdx, base: Reg, argc: u16, cache: CacheIdx }, // $o->m(...)
CallStatic { dst: Reg, class: ClassRef, name: NIdx, base: Reg, argc: u16, cache: CacheIdx }, // C::m(...) / self:: / static:: / parent::

// type test / copy
InstanceOf { dst: Reg, val: Reg, class: ClassRef },  // never throws
Clone      { dst: Reg, src: Reg },                   // shallow copy + new id + __clone

// lvalue cursor additions (§3.3)
LvalProp       { name: NIdx },                        // descend into property `name`
LvalStaticProp { class: ClassRef, name: NIdx },       // cursor ROOTED at a static-prop cell
```

`?` (may enter the unwinder) applies to `New`, `FetchProp`, `FetchStaticProp`,
`FetchClassConst`, `CallMethod`, `CallStatic`, `Clone`. `InstanceOf` never
throws. **No `AssignProp`** — writes go through the cursor.

Null-safe `$o?->p` / `$o?->m()` **do not** get their own opcodes: the compiler
lowers them to a `JmpNull` guard (reusing the `??` pattern) around the
`FetchProp`/`CallMethod`, short-circuiting the whole chain to `Null` and not
evaluating args. (See §3.5 lowering.)

### 3.3 Lvalue cursor extension (so `$o->p[...] = v` works)

Current VM: `cursor_root: Reg` + `cursor_steps: Vec<CursorStep>` where
`CursorStep ∈ {Key(Value), Append}`. Extend minimally:

```rust
enum CursorStep {
    Key(Value),
    Append,
    Prop(PhpStr),              // NEW: descend into object property (case-sensitive)
}
enum CursorRoot {              // NEW: the cursor may root at a CV or a static-prop cell
    Cv(Reg),                   // existing behavior (LvalCv)
    Cell(Rc<RefCell<Value>>),  // static-prop slot (LvalStaticProp)
}
// Vm.cursor_root: Reg  ->  Vm.cursor_root: CursorRoot
```

- `LvalCv { cv }` sets `Cv(cv)` (unchanged behavior).
- `LvalProp { name }` pushes `CursorStep::Prop(name)`.
- `LvalStaticProp { class, name }` resolves the class, grabs the static slot's
  `Rc<RefCell<Value>>`, sets `Cell(rc)`.
- `AssignLval`/`FetchLvalRw`/`UnsetLval`/`IncDecLval` are **unchanged in shape** —
  they dispatch on the root: `Cv(r)` → `&mut regs[r]` as today; `Cell(rc)` →
  `&mut *rc.borrow_mut()`.

The path recursion (`assign_path`/`read_path`/`unset_path`) gains **one arm**:
`(Value::Object(handle), CursorStep::Prop(name))` borrows the handle mut and
descends into `properties.entry(name).or_insert(Null)`, then recurses on the
tail. **Crucially — no COW at an object level** (handles share mutation); an
array *inside* a property still COWs via the existing `(Array, Key)` arm. The two
disciplines compose: object levels share, array levels COW.

```rust
// in assign_path(slot: &mut Value, steps, value)
(Value::Object(handle), CursorStep::Prop(name)) => {
    let mut od = handle.borrow_mut();
    let child = od.properties.entry(name.clone()).or_insert(Value::Null);
    assign_path(child, rest, value)        // SAME recursion for the tail
}
(Value::Null, CursorStep::Prop(_))  => Err(Thrown::error("Error","Attempt to assign property on null")),
(scalar,      CursorStep::Prop(_))  => Err(Thrown::error("Error","Attempt to assign property on <type>")),
```

`&`-bound property values are `Value::Ref`, caught by the existing `(Ref, _)`
arm — reference machinery reused unchanged.

Compiler: `collect_lvalue_steps` gains `Access::Property` (push `Prop(name)`,
recurse into `object` for the root) and `Access::StaticProperty` (emit the cursor
with `LvalStaticProp` as root instead of `LvalCv`). Everything after
(`AssignLval`/`FetchLvalRw`/`IncDecLval`) is the existing compound-assign/inc-dec
code byte-for-byte.

### 3.4 Inline-cache slot plan (reserved, `ClassId`-guarded, monomorphic)

(Not built. The reserved `InlineCache` table and the `cache` operands were
never read and have been removed; lookups probe the flattened class table.)

`InlineCache { class_id: u32, payload: u32 }` already exists (`Cell`-based, no
atomics). Every IC-bearing opcode carries `cache: CacheIdx` from day one so the
ISA shape never changes when the IC is filled. Guard-and-fill, same every site
(store `class_id + 1` so `0 == empty`):

```rust
let ic = &proto.caches[cache as usize];
let slot = if ic.class_id.get() == cls + 1 { ic.payload.get() }            // HIT
           else { let x = self.resolve_X(cls, name)?;
                  ic.class_id.set(cls + 1); ic.payload.set(x); x };        // MISS -> same resolver
```

- `FetchProp`: `payload` = property index in the class's `props` IndexMap
  (`get_index`), ready to become the Zend `OBJ_PROP` slot later with no ISA
  change.
- `CallMethod`/`CallStatic`: `payload` = index into a resolved-method side vector
  (`Rc<FunctionProto>` + `defining_class` + `is_static`), skipping the `methods`
  hash probe.
- `New`/`FetchStaticProp`/`FetchClassConst`: `payload` = resolved `ClassId` for a
  `ClassRef::Name` only. **Never cache `self`/`parent`/`static`/`Dynamic`** (they
  depend on the frame or a runtime value — the guard would always miss).

Miss always calls the exact resolver the cold path uses: the IC is pure speedup,
never a second source of semantics.

### 3.5 Per-opcode VM semantics

- **`New`**: `resolve_class`; if `kind != Class || flags.is_abstract` → `Error
  "Cannot instantiate {abstract class|interface|trait} C"`. Alloc `ObjectData {
  class, id: next_object_id++ , properties: <materialized from entry.props in
  order> }` (each `PropDefault::Value` cloned per-instance so arrays COW
  independently; `Init` thunk run bound to the class; `Null` → `Value::Null`).
  Write the handle to `dst` **first**, then if `constructor.is_some()` enter it
  (`$this` = handle, `called_class = id`, `defining_class = ctor.defining_class`),
  discarding the return. A throw in the ctor unwinds normally (dst holds the
  orphan handle, as PHP does). No user ctor → surplus args ignored.
- **`FetchProp`**: deref `obj`; non-object → warning `Attempt to read property
  "p" on <type>` + `Null`. Object: `properties.get(name)` cloned out; absent → if
  class has `__get`, call it (normal path) and use its return; else warning
  `Undefined property C::$p` + `Null`.
- **`FetchStaticProp`**: `resolve_class`; `static_props.get(name)` →
  `cell.borrow().clone()` (run `init` thunk once on first use); absent → `Error
  "Access to undeclared static property C::$p"`.
- **`FetchClassConst`**: `resolve_class`; `name == "class"` → the class's name as
  `Value::Str` (works for `static::class`, `$o::class`); else resolve+memoize the
  `ClassConst` (`Resolving` re-entry → fatal `Error`); absent → `Error "Undefined
  constant C::K"`.
- **`CallMethod`**: deref `obj`; non-object → `Error "Call to a member function
  m() on <type>"`. Resolve `m` on the object's class (flattened `methods`);
  absent → `__call` (args packed to an array) or `Error`. Visibility check (§5).
  Push frame: `$this` = handle, `called_class = object's class`, `defining_class =
  method.defining_class`.
- **`CallStatic`**: `resolve_class`; resolve `m`; absent → `__callStatic` or
  `Error`. `called_class` per the §4 forwarding table; `$this` forwarded when the
  current `$this instanceof` target and the callee is non-static (baseline may
  bind no `$this` for a true static and revisit).
- **`InstanceOf`**: deref `val`; non-object → `Bool(false)`. `Bool(resolve_class
  (class).map(|c| classes[val.class].ancestors.contains(&c)).unwrap_or(false))`.
- **`Clone`**: deref `src` → handle; build `ObjectData { class, id:
  next_object_id++ , properties: od.properties.clone() }` (shallow: nested
  arrays/objects follow their own copy/handle discipline, so a clone *shares*
  nested object handles). New `Rc`, write to `dst`, run `__clone` (`$this` = new
  handle) if defined. `===` of clone vs original is false.

Lowering sketches (AST → bytecode), `tN` = temp:

```
$o = new Counter(5);     => LoadInt t0,5; New t1,Name(Counter),base=t0,argc=1; Assign cv(o),t1
$o->p = 1;               => LoadInt t0,1; LvalCv cv(o); LvalProp name("p"); AssignLval t0
$o->items[$i] += 2;      => <$i->t0>; LvalCv cv(o); LvalProp name("items"); LvalKey t0;
                            FetchLvalRw t1; Add t2,t1,<k 2>; AssignLval t2
$sum = $a->add(3)->total(); => LoadInt t0,3; CallMethod t1,obj=cv(a),name("add"),base=t0,argc=1,#3;
                               CallMethod t1,obj=t1,name("total"),base=t1,argc=0,#4; Assign cv(sum),t1
echo $o?->addr?->city;   => Mov t0,cv(o); JmpNull t0,Lnull; FetchProp t1,t0,name("addr"),#1;
                            JmpNull t1,Lnull; FetchProp t0,t1,name("city"),#2; Jmp Lend;
                            Lnull: LoadNull t0; Lend: Echo t0
Config::$count += 1;     => LoadInt t4,1; LvalStaticProp Name(Config),name("count");
                            FetchLvalRw t5; Add t6,t5,t4; AssignLval t6
static::make();          => CallStatic t2,StaticClass,name("make"),base=t2,argc=0,#7  ; called_class forwarded
echo $x instanceof Throwable; => InstanceOf t7,val=cv(x),Name(Throwable); Echo t7
$copy = clone $obj;      => Clone t0,src=cv(obj); Assign cv(copy),t0
```

---

## 4. Late static binding

`Frame` gains two `Option<ClassId>` (and keeps `$this` in its CV via
`proto.this_cv`):

```rust
struct Frame {
    proto: Rc<FunctionProto>, regs: Vec<Value>, pc: usize, ret_dst: Reg,
    defining_class: Option<ClassId>,  // self:: / parent:: / __CLASS__
    called_class:   Option<ClassId>,  // static:: / new static / static::class (LSB)
}
```

Resolution:
- **`self::`** → `defining_class`.
- **`parent::`** → `classes[defining_class].parent` (a *static*, non-virtual call —
  runs the ancestor's body even though the object is a subclass).
- **`static::`** → `called_class`.

How the two are set when a frame is pushed (verified against `php` 8.5.8):

| Call form | new `called_class` | new `defining_class` | `$this` |
|---|---|---|---|
| `new C()` / `new static()` | resolved `C` / caller's `called_class` | ctor's `defining_class` | the new object |
| `$o->m()` (`CallMethod`) | **`$o`'s runtime class** | `m`'s `defining_class` | `$o` |
| `C::m()` / `$x::m()` (non-forwarding) | resolved class | `m`'s `defining_class` | forwarded if compatible, else none |
| `self::m()` / `parent::m()` / `static::m()` (**forwarding**) | **caller's `called_class`** (preserved) | `m`'s `defining_class` | forwarded `$this` |

One line: **instance calls and `ClassName::`/`$x::` static calls SET the
called-class; `self::`/`parent::`/`static::` calls FORWARD it.** `{main}` and free
functions carry `None` for both; `static::` there is a compile-time error.

Probes (→ fixtures):

```php
class A { static function create(){ return new static(); }
          function who(){ return static::class; }
          static function sName(){ return self::class; }
          static function viaName(){ return A::create(); }        // non-forwarding
          static function viaStatic(){ return static::create(); } }// forwarding
class B extends A {}
B::create()   => instance of B    (new B)->who() => "B"    B::sName() => "A"
B::viaName()  => "A"              B::viaStatic() => "B"

class A2 { function id(){ return static::class; } }
class B2 extends A2 { function id(){ return "B+".parent::id(); } }   // parent:: keeps static::
class C2 extends B2 {}
(new C2)->id() => "B+C2"
```

---

## 5. Visibility — enforcement level

**Decision: parse + store + enforce**, with a caller-class check. Every
`PropEntry`/`ResolvedMethod`/`StaticProp`/`ClassConst` carries `Visibility` +
`declaring_class`. On each non-public access the VM checks the caller class =
`frame.defining_class`:

- **public** — always allowed.
- **private** — allowed iff `caller == declaring_class`.
- **protected** — allowed iff `caller` is-a `declaring_class` (same class, or one
  is an ancestor of the other) — cheap membership test on `ancestors` / `parent`.

Violation → matching `Error` (`Cannot access {private|protected} property C::$p`,
or `Call to {private|protected} method C::m()`).

Verified:
```php
class A { private $s="secret"; public function get(){ return $this->s; } }
$a = new A; $a->get();  // "secret"   $a->s;  // Error: Cannot access private property A::$s
```

Private-name shadowing is modelled with php's mangled storage keys
(`\0A\0x` vs `\0B\0x`, see oop-model.md §4). Documented simplification (cut 1):
**Typed-uninitialized** properties
materialize as `Null` (we store no type hints yet). *(Baseline shortcut allowed
during bring-up: land method/property plumbing with everything treated public,
then switch the check on — but the metadata must be stored from step 1.)*

---

## 6. Linking: compiler → `Vm::install` (flatten in dependency order)

`Vm::install` links each `ClassProto` into a `ClassEntry`:

1. **Topological order** by dependency (`parent`, each interface, each used trait
   must link first). A dependency cycle (`class A extends A`, mutual `extends`) →
   fatal `Error`. A missing name → `Error "Class \"X\" not found"`.
2. **Assign `ClassId`** = index in `Vm.classes`; record `class_ids[lc(name)] = id`.
3. **Flatten, parent-first** (traits → parent overlay → interface-set union):
   - `methods` = clone `parent.methods`; merge trait methods (adopting the using
     class as `defining_class`); insert own methods (lowercase key), **overriding**
     inherited. Enforce `final` (override of a `final` method → fatal), check
     abstract-implementation for a concrete class.
   - `props` = clone `parent.props`; for each own `PropProto`, replace **in place**
     if the name exists (new default/visibility/declaring_class), else append —
     reproducing the observed order.
   - `constants` = clone `parent.constants`; add/override own (`Resolved`/`Thunk`).
   - `static_props` = clone `parent.static_props` **cloning the `Rc` cells**
     (inherited statics alias); for each own static prop, install a **fresh**
     `Rc<RefCell<Value>>`.
   - `ancestors` = self ∪ parent's ancestors ∪ transitive interface closure.
   - cache `constructor` by resolving `__construct` in the flattened `methods`.

Static-prop and class-const inheritance semantics (→ fixtures):

```php
class A { public static $c = 0; } class B extends A {}
A::$c = 5; B::$c;  // 5  (B inherits A's SLOT)   B::$c = 9; A::$c; // 9 (shared storage)
class A2{public static $c=0;} class B2 extends A2{public static $c=0;}
A2::$c=1; B2::$c;  // 0  (B2 REDECLARES -> own slot)

class A3 { const FOO=1; const BAR=self::FOO+1; }
class B3 extends A3 { const BAZ=parent::FOO+10; }
A3::BAR; // 2   B3::BAZ; // 11   B3::FOO; // 1
```

---

## 7. OOP CUT 1 — ordered, cargo-stays-green checklist

Scope of cut 1: **classes with properties + methods + `__construct` + `$this` +
`->prop` read/write + `->method()` + `new` + `extends` + `parent::` +
`instanceof` + class constants + static props/methods + `self::`/`static::`.**
Each step compiles and keeps existing tests green; most add a `php ⟷ vm` fixture.

1. **Leaf types** (`rhp-values`): `ClassKind`, `Visibility`, `ClassFlags`.
   Confirm `ObjectData`/`ObjectHandle` unchanged. *Milestone:* `cargo build`, no
   behavior change.
2. **Bytecode data** (`rhp-bytecode`): add `FunctionProto.{this_cv, is_static}`;
   replace `Program.classes` placeholder with `Vec<ClassProto>` + the `*Proto`
   structs + `ClassRef`. *Milestone:* builds; existing protos set
   `this_cv=None,is_static=false`.
3. **Class table + install** (`rhp-vm`): `ClassEntry`/`ResolvedMethod`/
   `PropEntry`/`StaticProp`/`ClassConst`; add `Vm.{classes, class_ids,
   next_object_id}`; implement `Vm::install` flatten + topo link (classes + props +
   constants + static cells + ancestors + constructor; `final`/abstract/cycle/
   missing-name errors). *Milestone:* install a hand-built single class; unit-test
   the flatten (override in place, static-cell aliasing, const memoize).
4. **`New` + `$this` + instance methods + `__construct`**: compiler lowers
   `Class` declaration → `ClassProto` (methods as `FunctionProto` with
   `this_cv=Some(0)` for instance, params shifted; CV pre-pass seeds `cv_map[0] =
   "this"`); compiler lowers `Instantiation` → `New`; add `enter_method_call`
   (mirror `enter_user_call`, bind `$this` + set frame `called_class`/
   `defining_class`). *Milestone:* `new C(); $o->m()` where `m` uses `$this->p`
   round-trips vs php — a class with a ctor storing a promoted/explicit prop and a
   method reading it.
5. **`FetchProp` + property write via cursor**: `FetchProp` opcode + VM;
   `CursorStep::Prop` + `CursorRoot` + the `assign_path`/`read_path`/`unset_path`
   object arm; compiler `collect_lvalue_steps` `Access::Property`; `LvalProp`.
   *Milestone:* `$o->p`, `$o->p = v`, `$o->p[$i] = v`, `$o->p++`, nested
   `$o->a->b` vs php.
6. **`CallMethod`** with the monomorphic IC fill (property IC too). *Milestone:*
   chained `$a->f()->g()` + a hot loop calling one method (IC hit) vs php.
7. **`extends` + virtual dispatch + `parent::`**: multi-class `ClassProto`s;
   `CallStatic` with `ParentClass`/`SelfClass`; frame `defining_class` threading.
   *Milestone:* the §1.5 `A`/`B` `go()`/`hi()`/`parent::hi()` fixture.
8. **Static props/methods + `self::`/`static::` + LSB**: `FetchStaticProp`,
   `LvalStaticProp`, `CallStatic` with `Name`/`SelfClass`/`StaticClass`/`Dynamic`;
   the §4 forwarding table. *Milestone:* the §4 LSB probes + the §6 static-prop
   aliasing/redeclare fixtures.
9. **Class constants**: `FetchClassConst` (incl `::class`); lazy memoize + cycle
   detect. *Milestone:* the §6 `const`/`self::`/`parent::` fixture + `C::class`.
10. **`instanceof`**: `InstanceOf` opcode over `ancestors`. *Milestone:* subclass
    true, unrelated false, non-object false vs php (interfaces beyond basic
    `implements` membership may be thin here — see defer list).
11. **`clone` + `__clone`**: `Clone` opcode. *Milestone:* `clone` gives new id,
    `!==` original, independent properties, `__clone` runs, nested handle shared.
12. **Visibility enforcement**: flip on the §5 caller-class check across
    prop/method/static/const access. *Milestone:* private/protected access errors
    match php.
13. **Constructor property promotion** (if low-cost): compiler emits a `PropProto`
    for each promoted param + a top-of-ctor `AssignLval` store. *Milestone:*
    `__construct(public int $x)` vs php. *(Defer to cut 2 if the param-modifier
    plumbing is heavy.)*
14. **`DeclareClass`** for conditional declarations. *Milestone:* a class declared
    inside an `if` links at execution and is usable after.

**Defer to cut 2** (none reshape the above types): traits (basic `use` merge is
near-free at link but may slip), interfaces beyond `instanceof` membership,
abstract-method enforcement edge cases, **enums**, magic methods beyond
`__construct` (`__get`/`__set`/`__call`/`__callStatic`/`__toString`/`__invoke`),
`__clone` can be cut-1 cheap but formalize in cut 2, **readonly** enforcement,
property **hooks**, asymmetric visibility (`*Set`), trait adaptations
(`insteadof`/`as`), by-ref return, first-class callables / `PartialApplication`,
anonymous classes, attributes, private-name shadowing mangling, typed-uninit
sentinel, closures with bound `$this`/scope + `Closure::bind`. On encountering a
deferred surface, emit a structural `Refuse` (not a catchable `Throw`) until
implemented.

---

## 8. Do-not-regret list (OOP-specific)

1. **Handle vs value at every cursor level.** A `Prop` step descends through
   `Rc<RefCell>` with **no `make_mut`** (shared mutation); a `Key`/`Append` step
   inside a property still COW-clones the array. Each is caught by its own
   `assign_path` arm. Mixing them up turns objects into value types or arrays into
   handles.
2. **`$this` binding is a CV, set on method entry.** `enter_method_call` binds
   `regs[this_cv] = handle` for instance methods; static methods bind none.
3. **Called-class vs defining-class.** `static::` → `called_class`;
   `self::`/`parent::` → `defining_class`. `CallMethod` sets `called_class` to the
   object's *runtime* class; `self::`/`parent::`/`static::` *forward* it. Get the
   §4 forwarding table wrong and every LSB fixture diverges.
4. **Property defaults cloned per instance.** `New` materializes each
   `PropDefault::Value` with `.clone()` so two objects of one class never alias a
   property; the clone is cheap (`Rc` bumps) but must happen.
5. **One shared static cell per class.** Inherited-but-not-redeclared statics
   alias the ancestor's `Rc<RefCell<Value>>`; redeclaration makes a fresh cell. The
   cursor roots at the cell so `C::$s[$i] = v` writes the shared slot.
6. **Flattened-table correctness on override.** A child method overrides by
   lowercased name but keeps its own `defining_class`; a redeclared property
   replaces the entry **in place** (preserving position + updating default). Verify
   `var_dump` order and `parent::` resolution after override.
7. **Object ids are php's handles**, taken on every `New`, `Clone`, closure
   and generator and reused once freed. `===` is `Rc::ptr_eq` (a fresh `Rc`
   per object is identity); `id` is for `spl_object_id`/hashing.
8. **Magic methods go through the normal call path.** `__construct`/`__get`/
   `__call`/`__callStatic`/`__clone` push ordinary frames with `$this` + LSB set —
   no special re-entrancy; a throw inside unwinds like any call.
9. **ICs guard on `ClassId` only, store `+1`** (`0 == empty`). Never cache
   `self`/`parent`/`static`/`Dynamic`. Miss calls the same resolver — pure speedup.
10. **Cycles leak (for now).** `$o->self = $o`, object-holding static props/consts
    are `Rc` cycles the deferred mark-and-sweep cleans later; these types don't
    change when it lands. Don't create a borrow that deadlocks on a self-referential
    property during one `borrow_mut` chain.
```
