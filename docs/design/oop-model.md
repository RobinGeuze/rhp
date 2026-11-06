# rhp OOP runtime object + class model

Status: **design, phase 7 (OOP, VM-only).** Target semantics: PHP 8.5 (ground
truth: the reference `php`, see `crates/rhp-builtins/src/version.rs`; probed on
8.5.8). This doc fixes the concrete Rust types for the runtime **class table**
and **object model**, extending the
`value-model.md` §5 object sketch, and says how `FunctionProto`/`Program`/`Frame`
grow to carry methods. It is the authoritative spec for the class/object runtime;
the OOP **opcode** set (`New`, `FetchProp`, `CallMethod`, …) is sketched in §9 to
pin the calling convention but is specified in full alongside the compiler work.

Non-negotiable invariants (unchanged from `bytecode.md`/`value-model.md`):
single-threaded `Rc`/`RefCell` (never `Arc`); `Value`s are owned `'static` — an
object carries a `ClassId(u32)`, and the class table lives **in the `Vm`**, never
in a `Value`; opcodes delegate to `ops::`/`conv::`/`registry()` where applicable;
the ISA stays JIT-lowerable (fixed `Copy` operands, side tables, `ClassId`-guarded
inline caches). Objects are **handle** types (`Rc<RefCell<ObjectData>>`):
assignment shares the handle, `clone` copies. The deferred cycle collector still
applies — object graphs cycle (`$o->self = $o`, a static prop holding an object)
and `Rc` leaks them until mark-and-sweep lands; none of the types below change
when it does.

All behavior claims are verified against `php` 8.5.8 and reproduced inline so they
become fixtures.

---

## 0. Decisions (TL;DR)

1. **Flatten at class-registration (link) time**, not walk-the-chain at call
   time. Each `ClassEntry` holds *complete* method / property / constant / static
   tables (parent merged first, then self overrides). Dispatch is one hash lookup
   (methods) or one index (properties) — O(1), and a `ClassId`-guarded inline
   cache can cache the resolved slot. This mirrors Zend, which copies the parent's
   function/property tables into the child at link time. §1.
2. **Classes are linked by the `Vm`, in dependency order.** The compiler emits an
   un-linked `ClassProto` per declaration into `Program.classes` (parent /
   interfaces / traits referenced *by name*). `Vm::install` topologically links
   them into `Vec<ClassEntry>` (parent + interfaces + used traits first),
   assigning `ClassId`s and building the flattened tables. Conditional
   (non-top-level) declarations use a `DeclareClass` opcode, like `DeclareFn`. §6.
3. **`FunctionProto` stays `ClassId`-free.** It grows `this_cv: Option<Reg>` and
   `is_static: bool` only. The *defining class* is carried by the `MethodEntry`
   (resolved at link time) and propagated into the `Frame`, so a proto compiled
   before `ClassId`s exist needs no fixups. §5.
4. **Late static binding needs two class ids in the `Frame`:** `defining_class`
   (→ `self::`, `parent::`, `__CLASS__`) and `called_class` (→ `static::`,
   `new static`, `static::class`). §3.
5. **Static-property slots are shared `Rc<RefCell<Value>>`.** An inherited static
   prop aliases the parent's cell (writes seen through both names); a redeclared
   one gets a fresh cell. Verified §4.
6. **Visibility is parsed, stored, and enforced** with a simple caller-class
   check (public / private / protected). §4. Private-name shadowing uses php's mangled
   storage keys (§4). Documented simplification: **typed-uninitialized** properties are
   treated as `Null` (we store no type hints yet, so we cannot distinguish them).
   §2, §8.

---

## 1. The class table: `ClassEntry` (lives in the `Vm`)

```rust
// rhp-values (leaf): small Copy data shared by compiler + VM.
pub type ClassId = u32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClassKind { Class, Interface, Trait, Enum }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Visibility { Public, Protected, Private }

#[derive(Clone, Copy, Default, Debug)]
pub struct ClassFlags {
    pub is_abstract: bool,
    pub is_final: bool,
    pub is_readonly: bool,   // class-level readonly (8.2+)
}
```

```rust
// rhp-vm: the linked, ready-to-run class. One per declared class/interface/
// trait/enum, indexed by ClassId in `Vm.classes`. All tables are FLATTENED.
pub struct ClassEntry {
    pub id: ClassId,
    /// Original-case name (for messages / get_class / ::class).
    pub name: PhpStr,
    pub kind: ClassKind,
    pub flags: ClassFlags,

    /// Direct parent (`extends`). `None` for a root class / interface / trait.
    pub parent: Option<ClassId>,
    /// All interfaces this class satisfies, TRANSITIVELY flattened
    /// (own `implements` ∪ parent's ∪ each interface's own `extends`).
    /// Drives `instanceof` / `catch` matching in O(1) via a membership check.
    pub interfaces: Vec<ClassId>,

    /// FLATTENED virtual method table: lowercase method name -> entry.
    /// Inherited entries are copied in from the parent; an own method with the
    /// same (lowercased) name overrides. Method names are case-insensitive.
    pub methods: HashMap<Vec<u8>, MethodEntry>,

    /// FLATTENED instance-property descriptors, in PHP declaration order:
    /// parent's properties first (keeping their positions), then this class's
    /// NEW properties appended. An override updates the descriptor *in place*
    /// at the inherited position (verified §2). Property names are
    /// case-sensitive.
    pub properties: Vec<PropEntry>,
    /// name -> index into `properties`, for O(1) `$o->p` resolution.
    pub property_index: HashMap<PhpStr, usize>,

    /// FLATTENED class constants (case-sensitive). Lazily resolved + memoized
    /// (initializers may reference `self::`/`parent::`/other consts). §4.
    pub constants: HashMap<Vec<u8>, ClassConst>,

    /// FLATTENED static-property slots. An inherited slot SHARES the parent's
    /// `Rc<RefCell<Value>>` (aliasing); a redeclared one is a fresh cell. §4.
    pub static_props: HashMap<Vec<u8>, StaticProp>,

    /// Resolved `__construct` (index key into `methods`), cached at link time;
    /// `None` => default (no-arg) constructor.
    pub constructor: Option<Rc<FunctionProto>>,
}
```

```rust
pub struct MethodEntry {
    pub proto: Rc<FunctionProto>,
    /// The class whose BODY defined this method. Drives `self::`/`parent::`/
    /// `__CLASS__`/`__METHOD__` inside it, and is distinct from the object's
    /// runtime class (the "called class"). For an inherited method this stays
    /// the ancestor that declared it; for a trait method it is the USING class
    /// (PHP: a trait method behaves as if written in the using class).
    pub defining_class: ClassId,
    pub visibility: Visibility,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_final: bool,
}

pub struct PropEntry {
    pub name: PhpStr,                 // case-sensitive, no `$`
    pub default: PropDefault,
    pub visibility: Visibility,
    pub is_readonly: bool,
    /// Declaring class — for visibility checks and (future) private shadowing.
    pub declaring_class: ClassId,
}

/// The per-instance initial value of a declared property.
pub enum PropDefault {
    /// Constant default, folded at compile time (`public $x = 1`, `= [1,2]`).
    Value(Value),
    /// Non-constant initializer (`public $x = self::F + 1`, `= FOO`, `= 1 << 3`).
    /// A zero-param thunk `FunctionProto` evaluated per-instance at `new` time,
    /// in a frame whose defining/called class is this class (so `self::` works).
    Init(Rc<FunctionProto>),
    /// Declared without a default. In full PHP a *typed* such property is
    /// UNINITIALIZED (accessing before write throws); an *untyped* one defaults
    /// to Null. This cut stores no type hints, so both are materialized as Null
    /// (see §8 for the faithful follow-up).
    Null,
}

pub struct StaticProp {
    /// Shared slot. Inherited statics clone THIS `Rc` (alias the same storage);
    /// a redeclaration installs a fresh `Rc`. §4.
    pub cell: Rc<RefCell<Value>>,
    pub visibility: Visibility,
    pub declaring_class: ClassId,
    pub is_readonly: bool,
}

/// A class constant, resolved lazily and memoized (so `const B = self::A + 1`
/// works regardless of textual order).
pub struct ClassConst {
    pub visibility: Visibility,
    pub declaring_class: ClassId,
    pub state: RefCell<ConstState>,
}
pub enum ConstState {
    Resolved(Value),
    /// Zero-param thunk; first read evaluates it, stores `Resolved`, and detects
    /// re-entrancy (self-referential constant) as a fatal `Error`.
    Thunk(Rc<FunctionProto>),
    Resolving, // guard sentinel for cycle detection during evaluation
}
```

### 1.1 Why flatten at link time (not walk the chain)

Verified method/property/constant semantics the flattened tables must reproduce:

```
class A { public function hi(){return "A::hi";} public function go(){return $this->hi();} }
class B extends A { public function hi(){return "B::hi + ".parent::hi();} }
(new B)->go();            // => "B::hi + A::hi"   (virtual dispatch on B; parent:: is static)
```

- **Dispatch is O(1) and JIT-cacheable.** `$o->m()` is `classes[o.class].methods[lc(m)]`
  — one lookup. A monomorphic inline cache (not built; the operands once
  reserved for it are gone, see `bytecode.md`) would guard on the object's
  `ClassId` and cache the resolved `Rc<FunctionProto>` / property index; a walk-the-chain
  model would make the cached payload depend on the whole ancestry and defeat the
  single-`ClassId` guard.
- **It matches Zend**, which copies the parent's `function_table` /
  `properties_info` into the child at link time; we are deliberately isomorphic so
  the VM and a future JIT reason about the same flat shape.
- **Cost is paid once.** Flattening is O(total members) per class at link; the
  alternative pays O(depth) on every call and every property touch — the hot path.
- **Downside, handled:** memory (each class stores its full tables) and a link
  **ordering constraint** — a parent must be linked before its child. Resolved by
  topological linking in `Vm::install` (§6); a cycle (`class A extends A`, mutual
  `extends`) is a fatal `Error` at link.

Walk-the-chain is kept only as the **cache-miss fallback inside the linker**
(resolving one name by climbing `parent`), never on the execution hot path.

---

## 2. Object instantiation

`ObjectData` is the §5 sketch, unchanged in shape:

```rust
// rhp-values
pub struct ObjectData {
    pub class: ClassId,
    pub id: u32,                                    // spl_object_id
    pub properties: IndexMap<PhpStr, Value, FxBuildHasher>,
}
pub type ObjectHandle = Rc<RefCell<ObjectData>>;    // Value::Object(ObjectHandle)
```

`new C(args...)` (opcode `New`, §9) executes:

1. **Instantiability check.** If `classes[cid].kind != Class` or
   `flags.is_abstract`, raise `Error: Cannot instantiate {abstract class|interface|trait} C`.
   Verified:
   ```
   abstract class A {}  new A;   // Error: Cannot instantiate abstract class A
   ```
2. **Fresh id.** `id = self.alloc_object_id()`: a handle from
   `rhp_values::handles`, php's object store. A freed object (its `Rc` dropped)
   gives its handle back, after what its properties held, and the next object
   takes the most recently freed one (`$a` id 1 → `unset($a)` → the next `new`
   is id 1 again). No reuse once shutdown destructors start. Closures and
   generators take handles too. Ids match php where objects die when php frees
   them: argument temporaries move into the call (`MovTake`), a method call's
   temporary object is released by the call. Objects that only php's cycle
   collector frees, and objects whose destructor rhp runs later than php, still
   give their handle back later than php does.
3. **Property initialization**, in `classes[cid].properties` order (parent-first,
   verified below): for each `PropEntry`, insert `name -> v` where `v` is
   `default.clone()` (`PropDefault::Value`), the result of running the `Init`
   thunk (bound to `cid` for `self::`), or `Value::Null` (`PropDefault::Null`).
   Each default `Value` is **cloned per instance** — a `Value::Array` default
   shares its `Rc` until first write (COW), a scalar is copied; two instances
   never alias each other's array property. Verified:
   ```
   class A { public $x = 1; public $arr = [1,2]; protected $p = "a"; }
   class B extends A { public $y = 2; public $x = 10; }
   // var_dump(new B) property order: x(=10), arr, p, y
   //   => inherited x/arr/p keep A's positions; x's VALUE is overridden in place;
   //      B's new y is appended.
   $o = new B; $o->arr[] = 3;  (new B)->arr === [1,2]   // per-instance COW, not shared
   ```
4. **Construct.** If `classes[cid].constructor` is `Some(proto)`, enter it as a
   method frame (§3) with `$this` = the new handle and the `new` arguments;
   otherwise ignore args beyond arity rules. Constructor-promoted parameters
   (`__construct(public int $x)`) are lowered by the compiler to a property store
   at the top of `__construct` (the param's `PropEntry` is emitted into the class
   like any declared property; the promoted store is ordinary `AssignProp`).
5. The `New` opcode's result register receives `Value::Object(handle)`.

`clone $o` (opcode `Clone`): new `id`, new `Rc<RefCell<..>>`, `properties` cloned
entry-by-entry (each `Value` cloned — shallow in PHP's sense: nested arrays/objects
share via `Rc`), then `__clone` is invoked on the copy if defined. This is the one
place an object "copies"; identity `===` of the clone vs original is false.

---

## 3. Method calls, `$this`, and `self`/`static`/`parent`

Method calls reuse the existing explicit `Vec<Frame>` call stack and the
`enter_user_call` machinery; a method is just a `FunctionProto` with a `$this`
slot and two class ids threaded into its frame.

### 3.1 `$this` binding convention

For an **instance** method the compiler reserves **CV 0 for `$this`**; declared
parameters take CV 1, 2, … (`ParamInfo.cv` shifted by one), and the proto records
`this_cv = Some(0)`, `is_static = false`. The CV pre-pass seeds `cv_map[0] = b"this"`
before scanning params, so a `$this` `DirectVariable` in the body naturally
resolves to CV 0. A **static** method has `this_cv = None`, `is_static = true`,
and params start at CV 0 as for free functions.

### 3.2 Frame growth

```rust
// rhp-vm
struct Frame {
    proto: Rc<FunctionProto>,
    regs: Vec<Value>,
    pc: usize,
    ret_dst: Reg,
    // -- OOP (None for a plain free function) --
    /// Class whose body defined the running method -> `self::`, `parent::`,
    /// `__CLASS__`. Equals the `MethodEntry.defining_class` of the callee.
    defining_class: Option<ClassId>,
    /// Late-static-binding class -> `static::`, `new static`, `static::class`.
    called_class: Option<ClassId>,
}
```

Entering a method (a new `enter_method_call`, parallel to `enter_user_call`):
binds `$this` into `regs[this_cv]` (for instance methods), binds args to CV
`this_cv+1..`, and sets `defining_class = Some(method.defining_class)`,
`called_class = Some(runtime_class)`.

### 3.3 Resolving `self` / `static` / `parent` (late static binding)

Verified:
```
class A {
    public static function create(){ return new static(); }  // LSB
    public function who(){ return static::class; }
    public static function sName(){ return self::class; }
}
class B extends A {}
B::create()  => instance of B      // `new static` uses CALLED class
(new B)->who() => "B"              // `static::class` = called class
B::sName()   => "A"               // `self::class` = DEFINING class
```

- **`self::`** / `self::class` / `__CLASS__` → `frame.defining_class`.
- **`static::`** / `new static` / `static::class` → `frame.called_class`.
- **`parent::`** → `classes[frame.defining_class].parent` (a *static*,
  non-virtual call — `parent::hi()` runs A's `hi` even though the object is a B).
- **`called_class` forwarding:** a *forwarding* call (`self::m()`, `parent::m()`,
  `static::m()`) **preserves** the current `called_class`; a *non-forwarding*
  entry (`$obj->m()`, `C::m()`, `new`) sets `called_class` to the object's class
  / the named class. This is exactly PHP's LSB forwarding rule.

Instance dispatch `$o->m()` is virtual: resolve in `classes[o.borrow().class].methods`,
set `called_class = o.class`, `defining_class = entry.defining_class`. Static
dispatch `C::m()` resolves in `classes[C].methods` with `called_class = C`
(unless `C` is `static`/`self`/`parent`, which forward per above).

---

## 4. Static properties, class constants, visibility

### 4.1 Static properties — shared cells, inheritance-aware

Verified:
```
class A { public static $count = 0; } class B extends A {}
A::$count = 5;  B::$count;   // => 5   (B inherits A's SLOT)
B::$count = 9;  A::$count;   // => 9   (same storage, written through both names)

class A { public static $c = 0; } class B extends A { public static $c = 0; }
A::$c = 1;  B::$c;           // => 0   (B REDECLARES -> its own slot)
```

Modeled directly by `StaticProp.cell: Rc<RefCell<Value>>`: at link time a child
clones the parent's `StaticProp` (sharing the `Rc`); a redeclared static prop
installs `Rc::new(RefCell::new(init))`. Access opcodes `FetchStaticProp` /
`AssignStaticProp` read/write `*cell.borrow()` — the §3 ref-aware discipline
applies to the cell exactly as to `Value::Ref`. Lazy first-use initialization of
non-constant static initializers runs the init thunk once (guarded by a flag on
first access) bound to the declaring class.

### 4.2 Class constants — lazy, memoized, self/parent-aware

Verified:
```
class A { const FOO = 1; const BAR = self::FOO + 1; }
class B extends A { const BAZ = parent::FOO + 10; }
A::BAR; // 2     B::BAZ; // 11     B::FOO; // 1 (inherited)
```

A literal constant is folded to `ConstState::Resolved(Value)` at link time. A
constant whose initializer references other constants / `self::` / `parent::` /
`1 << 3` is stored as `ConstState::Thunk(proto)` and resolved on **first access**:
flip to `Resolving` (re-entry while `Resolving` ⇒ fatal `Error` for a
self-referential constant), evaluate the thunk in a frame bound to `declaring_class`,
store `Resolved(v)`, and memoize. Lazy resolution sidesteps any link-time ordering
problem between constants and matches PHP's on-demand evaluation.

### 4.3 Visibility — enforce with a caller-class check

Verified:
```
class A { private $s="secret"; protected $p="prot"; public $pub="ok";
          public function get(){ return $this->s; } }
$a = new A;  $a->pub;                 // "ok"
$a->s;  // Error: Cannot access private property A::$s
```

**Decision: parse + store + enforce.** Every `PropEntry`/`MethodEntry`/
`StaticProp`/`ClassConst` carries a `Visibility` and `declaring_class`. On each
non-public access the VM checks the *caller class* = `frame.defining_class`:

- **public** — always allowed.
- **private** — allowed iff `caller == declaring_class`.
- **protected** — allowed iff `caller` is in an is-a relationship with
  `declaring_class` (same class, or either is an ancestor of the other). The
  flattened `parent` chain + `interfaces` set make this a cheap membership test.

A violation raises the matching `Error`
(`Cannot access {private|protected} property C::$p`, or `Call to {…} method`).
**Private-name shadowing** follows php exactly: an object's property map is
keyed by *storage key* — the plain name for public/protected, the mangled
`"\0Class\0name"` for private — so a parent's private `$x` and a child's `$x`
are separate slots (`ClassEntry.props`, in object-layout order). `$o->name` is
resolved per access against the calling scope (`classes::resolve_prop`, php's
`zend_get_property_offset`): the scope's own private wins, else the
most-derived declaration (`ClassEntry.prop_visible`), where a parent's private
is invisible (a dynamic property) and an own private / unrelated protected is an
access error naming the object's class. `get_object_vars` / `foreach` keep a slot
iff it is what its name resolves to from the scope; the dumpers, `(array)` and
`serialize` see php's mangled names (`classes::mangled_prop_key`).

Refinements, all following `zend_object_handlers.c`:

- **protected** is checked against the property's *prototype* class
  (`PropEntry.prototype`, the root of a redeclared public/protected chain), so
  a sibling may read a cousin's redeclared property.
- **Asymmetric visibility** (`private(set)` / `protected(set)`) lives in
  `PropEntry.set_visibility` and is enforced on every write path (`StoreProp`,
  cursor writes, compound assign, `++`, `&`, `unset`) with php's
  `Cannot (indirectly) modify|unset X(set) property C::$p from scope|global scope`.
- A name starting with NUL that is not declared is refused
  (`Cannot access property starting with "\0"`), so mangled private keys are
  unreachable by name.
- **BP_VAR_IS:** isset/empty/`??` compile their container chain with
  `FetchPropIs` / `FetchPropDynIs` / `FetchStaticPropIs`: inaccessible or missing
  is null without error, magic objects get `__isset` and then `__get`.
- **Magic indirect writes:** a cursor path (nested write, compound assign,
  `++`, reference) through a missing/inaccessible property of a class with an
  unguarded `__get` goes through `__get` (`MagicSplit`), writing into its
  by-ref result or a temporary — never creating a real property.

---

## 5. `FunctionProto` growth + how `Program` carries classes

### 5.1 `FunctionProto` (rhp-bytecode) — two new fields, no `ClassId`

```rust
pub struct FunctionProto {
    // ... all existing fields unchanged ...
    /// CV register holding `$this` for an instance method; `None` for a free
    /// function, a closure without `$this`, or a static method.
    pub this_cv: Option<Reg>,
    /// True for a `static function`; selects the no-`$this` binding path.
    pub is_static: bool,
}
```

The proto deliberately does **not** store its defining `ClassId`: protos are
compiled before linking assigns ids, and are `Rc`-shared. The defining class
travels with the `MethodEntry` (set at link) into `Frame.defining_class`, and
late static binding uses `Frame.called_class` — so `self::`/`parent::`/`static::`
resolve from the frame, and no proto needs a post-link fixup. `__METHOD__`/
`__FUNCTION__` read `proto.name`; `__CLASS__` reads `frame.defining_class`.

### 5.2 `Program` + `ClassProto` (rhp-bytecode) — the un-linked class

```rust
pub struct Program {
    pub main: Rc<FunctionProto>,
    pub functions: Vec<Rc<FunctionProto>>,
    pub classes: Vec<ClassProto>,   // compiled, UN-LINKED; linked by the Vm (§6)
}

/// One compiled class/interface/trait/enum declaration, parents referenced by
/// NAME (resolved to ClassId at link time). Data only — no Rc cells yet.
pub struct ClassProto {
    pub name: PhpStr,
    pub kind: ClassKind,
    pub flags: ClassFlags,
    pub parent: Option<PhpStr>,         // `extends` (single, for classes)
    pub interfaces: Vec<PhpStr>,        // `implements` / interface `extends`
    pub traits: Vec<PhpStr>,            // `use T;` members to merge in
    pub methods: Vec<MethodProto>,
    pub properties: Vec<PropProto>,
    pub constants: Vec<ConstProto>,
    pub static_props: Vec<StaticPropProto>,
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
    pub default: PropDefaultProto,      // Const(CIdx) | Init(Rc<FunctionProto>) | None
}
pub enum PropDefaultProto { Const(CIdx), Init(Rc<FunctionProto>), None }
pub struct ConstProto {
    pub name: PhpStr,
    pub visibility: Visibility,
    pub value: ConstInitProto,          // Const(CIdx) | Thunk(Rc<FunctionProto>)
}
pub enum ConstInitProto { Const(CIdx), Thunk(Rc<FunctionProto>) }
pub struct StaticPropProto {
    pub name: PhpStr,
    pub visibility: Visibility,
    pub is_readonly: bool,
    pub init: ConstInitProto,
}
```

The `CIdx` in a `*Proto` indexes the *consts* of whichever `FunctionProto` owns
the init thunk (or `main` for top-level folded literals); non-constant initializers
are compiled into zero-param `Rc<FunctionProto>` thunks that run on the VM like
any other proto, so there is **no second evaluator** — property/const/static
initializers reuse the compiler and VM wholesale.

Thunks (and attribute-argument thunks) carry `ProtoFlags::CONST_EXPR`. php
evaluates initializers with no frame of its own (`zend_ast_evaluate`), so
traces leave thunk frames out, and while one runs errors report its current
line (each sub-expression is compiled with its own line) as php's
`EG(filename_override)`/`EG(lineno_override)`. A trace built under the
override starts with a `[constant expression]()` frame at the real execution
point when that differs. A thunk that is just a global constant name, or
has no class scope (a function's attribute argument or parameter default),
is `CONST_EXPR_PLAIN` and sets no override, and an autoloader runs without one
(`zend_lookup_class`).

---

## 6. Linking: compiler → `Vm` (flatten in dependency order)

The compiler emits one `ClassProto` per `Statement::Class`/`Interface`/`Trait`/
`Enum`, parents by name, into `Program.classes` (mirroring how functions are
hoisted today). `Vm::install` links them:

1. **Topological order** by dependency (`parent`, each `implements`, each used
   trait must link first). A dependency cycle is a fatal `Error`. A missing
   parent/interface name is `Error: Class "X" not found`.
2. **Assign `ClassId`** = index in `Vm.classes`; record
   `class_ids: HashMap<Vec<u8>, ClassId>` keyed by **lowercased** name
   (class names are case-insensitive).
3. **Flatten**, parent-first:
   - `methods` = `parent.methods.clone()`; merge trait methods (adopting the
     using class as `defining_class`); then insert own methods (lowercase key),
     **overriding** inherited entries. Enforce `final` (overriding a `final`
     method is a fatal `Error`) and abstract-implementation rules.
   - `properties` = `parent.properties.clone()` (+ `property_index`); for each own
     `PropProto`, if the name already exists replace the `PropEntry` **in place**
     (new default/visibility/`declaring_class`), else append — reproducing the
     observed order (§2).
   - `constants` = `parent.constants.clone()`; add/override own (as
     `ConstState::Resolved`/`Thunk`).
   - `static_props` = `parent.static_props.clone()` — **cloning the `Rc` cells**
     (inherited statics alias the parent slot); for each own static prop, install
     a **fresh** `Rc<RefCell<Value>>` (redeclared statics get their own slot).
   - `interfaces` = transitive closure of own + parent's + each interface's.
   - cache `constructor` by resolving `__construct` in the flattened `methods`.

Conditional declarations (a class inside an `if`, not at top level) are not
hoisted: the compiler emits `DeclareClass { class_idx }`, and the VM links that
`ClassProto` on execution (its dependencies must already be linked, as in PHP).

`Vm` growth:
```rust
pub struct Vm {
    // ... existing: out, stack, functions, fn_names, constants, max_depth, cursor_* ...
    classes: Vec<ClassEntry>,
    class_ids: HashMap<Vec<u8>, ClassId>,   // lowercase name -> id
}
// Object handles: rhp_values::handles (thread-local; freed from Drop).
```

---

## 7. `instanceof`, `catch`, and object comparison

- **`instanceof` / `catch` matching** (`InstanceOf` opcode; and the §3.4
  exception unwinder's `matching_catch`): `o.class == target` OR `target` ∈
  `classes[o.class].interfaces` OR `target` is an ancestor via the `parent`
  chain. All O(1)/O(depth-at-link) using the flattened `interfaces` set + parent
  chain. Phase-7 objects let `class_of(&thrown)` return a real `ClassId`, so the
  unwinder's class-name matching (today string-based over the builtin hierarchy,
  `bytecode.md` §3.4) upgrades to id-based `instanceof` without an ISA change.
- **`===`** is `Rc::ptr_eq` on the two `ObjectHandle`s (equivalently equal `id`).
- **`==`** compares same class + equal property values (recursively, via the
  existing `ops::loose_eq`, extended with an `Object` arm that walks the two
  `properties` maps). Objects of different classes are never loosely equal.

---

## 8. Deferred / out-of-scope in this cut (noted, not forgotten)

- **Cycle collection.** `ObjectData.properties` can hold `Value::Object` handles
  back to the same object, and static-prop cells / class constants can root object
  graphs; `Rc` leaks cycles until the deferred mark-and-sweep collector lands. The
  types above are unchanged by it — the collector walks `ObjectData` + the `Vm`
  static-prop/const roots.
- **Typed-uninitialized properties.** Verified distinct state
  (`uninitialized(int)`, and `Error: must not be accessed before initialization`),
  but we store no type hints this cut, so `PropDefault::Null` materializes both
  typed-no-default and untyped-no-default as `Null`. Faithful follow-up: store the
  hint, add a `Value::Uninit` sentinel (a leaf `value-model` change) or track
  initialization out-of-band, and throw on read-before-write.
- **Trait adaptations** (`insteadof`/`as` renaming + visibility change): basic
  trait method/property merge is in; conflict-resolution adaptations are a
  follow-up.
- **Enums, interfaces-with-constants edge cases, `__get`/`__set`/
  `__call` magic, `readonly` enforcement, Closure `$this` rebinding**: layer onto
  these types later; none require reshaping the class table. Property hooks
  are in (§8.1).

### 8.1 Property hooks

Follows php's zend_compile_property_hooks and the hook branches of
zend_object_handlers.c.

- **Compile.** Each hook is a method proto named `$prop::get` / `$prop::set`
  (`ProtoFlags::HOOK`), with `$this` in CV 0; `get` returns the property type,
  `set` takes `$value` (or its declared parameter) and returns void. A short
  `get => e` is `return e`, a short `set => e` is `$this->prop = e`.
  `PropProto` carries `hooks: [Option<HookProto>; 2]`, `is_virtual`
  (zend_property_is_virtual: no hook body touches `$this->prop`),
  `is_abstract`, `is_final`. Interface properties are compiled too (always
  abstract and virtual). The declaration checks of zend_compile_prop_decl and
  zend_compile_property_hooks are compile errors (`FileCtx::fatal`).
  `parent::$prop::get()` compiles to `CallParentHook`.
- **Link.** `PropEntry` grows `hooks` (`Rc<[Option<ResolvedMethod>; 2]>`),
  `virt`, `is_final`, `is_abstract`; `ClassEntry::hooked` says some property
  has hooks. `inherit_prop` is do_inherit_property's hook part (final
  properties and hooks, a backed parent keeps the child backed, abstract
  hooks a backed property implements, set visibility); interface properties
  are merged like a parent's. `verify_hooked_props` is
  zend_verify_hooked_property, `verify_hook_types` the set-hook value type
  checks (a small subtype test, `type_is_subtype`, until a shared one
  exists), `verify_abstract_class` php's "contains N abstract methods" for
  methods and hooks. Link errors are fatal errors at the class's line
  (`link_fatal`).
- **Run.** Property access tests `ClassEntry::hooked` once; only then does it
  look for hooks (vm/src/hooks.rs). A hook runs unless the innermost frame is
  a hook of the same property (same prototype) on the same object
  (zend_should_call_hook); then the backing slot is used, or for a virtual
  property "Must not read from / write to virtual property". Virtual
  properties have no slot. Cursor paths (`$o->p[] =`, `$o->p .=`, `&$o->p`)
  treat a hooked property like a `__get` one (`MagicSplit`), with php's
  "Indirect modification of C::$p is not allowed" unless the get hook returns
  a reference or an object. `get_object_vars`, `json_encode` and `var_export`
  read hooked properties through their hooks (`Host::hooked_properties_for`);
  `var_dump`, `(array)` and `serialize` show the backing values. `foreach`
  over a hooked object walks a plan of the declared properties lazily, then
  the dynamic ones (zend_hooked_object_get_iterator).
- **Reflection.** `isVirtual`, `isAbstract`, `isFinal`, `hasHooks`,
  `getHooks`, `hasHook`, `getHook` (a ReflectionMethod for the hook) and the
  `PropertyHookType` enum (the prelude now compiles enums; each request gets
  its own case objects).

### 8.2 Lazy objects

Follows Zend/zend_lazy_objects.c and the lazy branches of
zend_object_handlers.c (vm/src/lazy.rs).

- **State.** `ObjectData` keeps rarely used state behind one boxed
  `extra` slot (the native payload and `LazyInfo`), so objects stay the same
  size. `LazyInfo` holds php's object flags (`UNINIT`, `PROXY`; both clear
  while the initializer runs, which guards re-entry), the info flags, the
  initializer, an initialized proxy's instance, and the storage keys of the
  slots still lazy (php's IS_PROP_LAZY). It goes away with the object.
- **Triggers.** A lazy object's declared slots are absent from
  `properties`, so accesses reach the absent-slot branches of props.rs and
  only there call `lazy_slot` (a lazy slot, or an undefined property with
  no unguarded magic method); the hit path is unchanged. `lazy_init`
  initializes (a ghost reverts its properties when the initializer throws,
  a proxy checks the factory's result) and the access repeats on the
  object it returns, holding the instance's magic guard as php does. Cursor
  paths split at such a step (`MagicSplit::Lazy`) and continue on the
  instance. get_properties users (`foreach`, `get_object_vars`,
  `json_encode`, `var_export`, `serialize`, `==`, clone) initialize through
  `Host::lazy_props_object` or `hooked_properties_for`; `(array)`,
  `get_mangled_object_vars` and `var_dump`/`print_r` do not, and an
  initialized proxy shows its instance.
- **Not done.** Typed reference sources (php's "Cannot assign string to
  reference held by property") and "Lazy object was released during
  initialization" (needs refcounts php's way). php's "Can not reset an
  object during property iteration" check is missing, and
  reset/next/current do not initialize a lazy object.

---

## 9. OOP opcodes (calling convention; full spec with the compiler work)

Same encoding and calling convention as `bytecode.md` §2 — fixed `Copy` operands,
side-table names, `ClassId`-guarded inline caches (`cache: CacheIdx`). Sketched
here to pin the shape the types above serve:

```
New          { dst, class: NIdx|ClassId, base, argc }     instantiate + run __construct; dst = handle
Clone        { dst, src }                                 shallow copy + new id + __clone
FetchProp    { dst, obj, name: NIdx, cache }              $o->p   (visibility-checked; cache: ClassId->prop index)
AssignProp   { obj, name: NIdx, src, cache }              $o->p = v  (ref-aware at the slot)
CallMethod   { dst, obj, name: NIdx, base, argc, cache }  $o->m(...)  virtual; sets called=obj.class
CallStatic   { dst, class, name: NIdx, base, argc, cache} C::m(...) / self:: / static:: / parent:: (forwarding rules §3.3)
FetchStaticProp  { dst, class, name: NIdx, cache }        C::$p
AssignStaticProp { class, name: NIdx, src, cache }        C::$p = v  (writes through the shared cell)
FetchClassConst  { dst, class, name: NIdx, cache }        C::K   (lazy-resolves + memoizes)
FetchClassConstDyn { dst, class, name: Reg }              C::{expr} (name must be a string)
FetchClassName   { dst, class, const_expr }               self/parent/static/$obj ::class (ZEND_FETCH_CLASS_NAME)
UnsetStaticProp  { class, name: Reg }                     unset(C::$p): resolve C, then throw
ConstExprProp    { obj, name: Reg }                       -> in a constant expression: enums only
ConstExprDim     { arr }                                  [] in a constant expression: "Cannot use [] on objects in constant expression"
ValueArgIsRef    { dst, callee, idx }                     is parameter idx of what $f calls by reference (SEND_FUNC_ARG)
InstanceOf   { dst, obj, class: NIdx|ClassId }            Bool via interfaces set + parent chain
```

`base`/`argc` reuse the contiguous arg-window convention; `CallMethod`/`CallStatic`
bind `$this` (instance) and set `Frame.{defining_class, called_class}` per §3.
`NullSafeMethod`/`NullSafeProperty` lower to a `JmpNull` guard that jumps to
the end of the chain's outermost link (property, method, static member or
dimension), whose result is set to null there (zend_short_circuiting_commit).
SEND_FUNC_ARG for a runtime callee asks `MethodArgIsRef`, `StaticArgIsRef`,
`FuncArgIsRef` or, for a callable value, `ValueArgIsRef`. `self`/`static`/`parent` as the `class`
operand are resolved from the current frame, not interned names.
