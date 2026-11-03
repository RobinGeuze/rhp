# rhp OOP ISA — classes, objects, methods (phase 7)

Status: **build spec (implement from this).** Target semantics: PHP 8.5 (ground
truth: the reference `php`, see `crates/rhp-builtins/src/version.rs`; every behavior below was probed
against it, originally 8.5.8). This is
the phase-7 continuation of [`bytecode.md`](../bytecode.md) — it adds the object
opcodes that §2.4 of that doc deferred ("`New`, `FetchProp`/`AssignProp`,
`CallMethod`, `CallStatic`, `InstanceOf`, `Clone`, late static binding are
**phase 7** — same encoding and calling convention; omitted here"). Everything in
`bytecode.md` still holds; this doc only *adds* variants and side tables, it
changes none.

**The differential gate is `php ⟷ vm` alone** (`bytecode.md` §6). That raises
the bar on getting the semantics right the first time, hence the probes inline.

Read first and reused verbatim: [`value-model.md`](../value-model.md) §5 (the
`ObjectData`/`ObjectHandle` model — **objects are handle types**,
`Rc<RefCell<ObjectData>>`, assignment shares, `clone` copies, `===` is
`Rc::ptr_eq`), [`ast-surface.md`](./ast-surface.md) §10 (OOP AST nodes), and the
already-built `Value`/`ObjectData` in `rhp-values` (§5 below).

---

## 0. Invariants carried in from `bytecode.md` (non-negotiable)

1. **Single-threaded `Rc`/`RefCell`**, never `Arc`. Objects are
   `Rc<RefCell<ObjectData>>`; the class table is `Rc`-shared read-only data in the
   `Vm`, never atomic.
2. **`Value`s stay owned `'static`.** An object `Value` carries only a
   `ClassId(u32)` + object `id(u32)` + its property map — **the class table lives
   in the `Vm`, not in the value** (`value-model.md` §5). No `Value` variant gains
   an `'arena` borrow.
3. **`Instr: Copy`, fixed-width, one variant per opcode.** Every operand is a
   small `Copy` index or immediate; all heap data (names, class protos, caches)
   lives in side tables on `FunctionProto`/`Program`. The new class operand
   (`ClassRef`, §2.1) is itself a small `Copy` enum — it owns no heap.
4. **Delegation rule.** Object opcodes contribute operand fetch, `?`-propagation
   of `Thrown`, and the `dst` write; everything semantic (method resolution,
   property lookup, `instanceof`) is a VM method the JIT can call on cold paths.
   New arithmetic is never introduced here.
5. **The lvalue cursor is extended, not forked.** `$o->p = v`, `$o->p[$i] = v`,
   `$o->p += v`, `$o->p++` all flow through the *same* `LvalCv`/`LvalKey`/
   `AssignLval` machinery that already exists (vm `lib.rs`), with one new step
   (`LvalProp`) and one new cursor root (`LvalStaticProp`).
6. **Inline caches are reserved side-table slots** (`bytecode.md` §2.5),
   `Cell`-based, monomorphic, `ClassId`-guarded. The `cache: CacheIdx` field is
   present on `New`/`CallMethod`/`FetchProp`/`CallStatic`/`FetchClassConst` from
   day one so the ISA shape never changes when the IC is actually filled.

Refusals unchanged: `eval`/backticks/`(unset)` stay structurally refused; a
`new class(...)` **anonymous class**, property **hooks**, and `enum` methods are
in-scope-later and may emit `Refuse` until implemented — never a catchable
`Throw`.

---

## 1. The class table (data in `rhp-bytecode`, resolved in the `Vm`)

`Program` grows the `classes` slot that is currently commented out in
`bytecode.md` §2.1 and `rhp-bytecode/src/lib.rs`:

```rust
pub struct Program {
    pub main: Rc<FunctionProto>,
    pub functions: Vec<Rc<FunctionProto>>,
    pub classes:   Vec<Rc<ClassProto>>,   // ClassId indexes this (phase 7)
}
```

### 1.1 Compile-time shape (`rhp-bytecode`, data only, no behavior)

```rust
pub type ClassId = u32;              // index into Program.classes / Vm.classes

pub struct ClassProto {
    pub name: PhpStr,                        // declared name (case-insensitive match later)
    pub kind: ClassKind,                     // Class | Interface | Trait | Enum
    pub flags: ClassFlags,                   // abstract, final, readonly
    pub parent: Option<NIdx>,                // `extends X` (single), resolved at link
    pub interfaces: Vec<NIdx>,               // `implements`/interface `extends` list
    pub uses: Vec<NIdx>,                     // `use TraitA, TraitB;` (flattened at link)
    pub consts: Vec<ClassConstDef>,          // const K = <const-expr>
    pub props: Vec<PropDef>,                 // instance property declarations
    pub static_props: Vec<PropDef>,          // static property declarations
    pub methods: Vec<MethodDef>,             // this class's own methods
    pub this_cv: Reg,                        // CV that $this binds to in methods (== cv("$this"))
}

pub struct MethodDef {
    pub name: PhpStr,
    pub fnidx: FnIdx,                        // the method body is a normal FunctionProto
    pub flags: MethodFlags,                  // static, abstract, final, visibility
}
pub struct PropDef  { pub name: PhpStr, pub default: Option<CIdx>, pub flags: PropFlags }   // default is a const; non-const inits use an Init(Pc) block (deferred)
pub struct ClassConstDef { pub name: PhpStr, pub value: CIdx }   // enum cases are const-like (deferred detail)

#[derive(Clone, Copy)] pub enum ClassKind { Class, Interface, Trait, Enum }
```

A method body is an ordinary `FunctionProto` in `Program.functions` (same calling
convention, same register model). The only method-specific addition is
`FunctionProto.this_cv: Option<Reg>` — the CV that receives `$this` on entry
(`None` for free functions / `{main}`). `$this` is thus an ordinary CV read with
no opcode, exactly like any other variable (`value-model.md`: `$this` is a
`DirectVariable` named `$this`).

### 1.2 Runtime shape (`rhp-vm`, built once at install)

At `Vm::install`, each `ClassProto` is linked into a `ClassEntry` with
inheritance **flattened** so every hot lookup is one hash probe with no
walk-the-parents loop:

```rust
pub struct ClassEntry {
    pub name: PhpStr,
    pub id: ClassId,
    pub kind: ClassKind,
    pub flags: ClassFlags,
    pub parent: Option<ClassId>,
    pub ancestors: Vec<ClassId>,                       // self + all transitive parents + interfaces (for instanceof)
    pub methods: HashMap<Vec<u8>, ResolvedMethod>,     // lowercased name -> method (flattened; child overrides parent)
    pub constants: HashMap<Vec<u8>, Value>,            // flattened class constants
    pub prop_template: IndexMap<PhpStr, Value>,        // instance props in declaration order, defaults materialized
    pub static_props: IndexMap<PhpStr, Rc<RefCell<Value>>>, // shared mutable slots (one per class, stable handle)
}

pub struct ResolvedMethod {
    pub proto: Rc<FunctionProto>,
    pub defining_class: ClassId,   // where the body is declared — drives self::/parent::
    pub is_static: bool,
    pub visibility: Vis,
}
```

- **`ancestors`** is the `instanceof` set: a precomputed transitive closure so
  `InstanceOf` is a membership test, never a parent walk.
- **`methods`** is flattened at link (parent methods copied in first, then the
  child's own override) and keeps each method's `defining_class`. That split —
  *the class an object is an instance of* vs *the class a method is declared in* —
  is the whole of late static binding (§4).
- **`static_props`** holds one `Rc<RefCell<Value>>` **per class** (inherited
  static props alias the ancestor's cell per PHP, unless redeclared). A stable
  cell handle lets the lvalue cursor root at it (§3.3).
- **`prop_template`** is the per-instance starting map; `New` clones it (every
  value clone is an `Rc` bump) so each object gets its own property storage.

`Vm` gains the fields reserved in `bytecode.md` §3.1:

```rust
classes: Vec<ClassEntry>,
class_ids: HashMap<Vec<u8>, ClassId>,   // lowercased name -> id
next_object_id: u32,                    // spl_object_id source, monotonically increasing
```

---

## 2. New `Instr` variants (concrete, `Copy`, side-table operands)

### 2.1 The class operand `ClassRef`

A class is named four ways in source (`Foo`, `self`, `parent`, `static`, and the
dynamic `$cls`/`new $obj`). All fit one `Copy` immediate — no heap:

```rust
#[derive(Clone, Copy, Debug)]
pub enum ClassRef {
    Name(NIdx),     // `Foo` — interned name; runtime resolves to a ClassId (IC-cached)
    SelfClass,      // `self`   — the current frame's DEFINING class
    ParentClass,    // `parent` — parent of the current frame's defining class
    StaticClass,    // `static` — the current frame's CALLED class (late static binding)
    Dynamic(Reg),   // `new $cls()` / `$cls::m()` — a runtime string|object in a register
}
```

Resolution lives in one VM helper `resolve_class(&self, top, ClassRef) ->
Result<ClassId, Thrown>`; every opcode below that names a class calls it. For
`Dynamic`, a `Value::Str` resolves by name and a `Value::Object` resolves to its
class (PHP's `$obj::CONST` / `new $obj`).

### 2.2 The object opcodes

Appended to the `Instr` enum (SCREAMING disassembly spelling in parentheses):

```rust
// -- instantiation (New) --
/// `new C(args)` / `new $cls(args)`. Allocate the object into `dst`, then run
/// __construct via the normal call path with the arg window [base, base+argc).
New { dst: Reg, class: ClassRef, base: Reg, argc: u16, cache: CacheIdx },

// -- property read (value context) --
/// `$o->p`. Object handle in `obj`, property `name`. Honors __get; warns + Null
/// on a missing prop / non-object (null-read warning). IC-guarded by ClassId.
FetchProp { dst: Reg, obj: Reg, name: NIdx, cache: CacheIdx },

// -- static property read (value context) --
/// `C::$p`. Resolve class, read the shared static slot (clone out).
FetchStaticProp { dst: Reg, class: ClassRef, name: NIdx, cache: CacheIdx },

// -- class constant / ::class --
/// `C::K` and `C::class` (name == "class" yields the resolved class name string).
FetchClassConst { dst: Reg, class: ClassRef, name: NIdx, cache: CacheIdx },

// -- method calls (push a frame via the normal calling convention) --
/// `$o->m(args)`. Resolve `name` on the object's class's flattened table, bind
/// $this + LSB (called = object's runtime class), push a frame. Honors __call.
CallMethod { dst: Reg, obj: Reg, name: NIdx, base: Reg, argc: u16, cache: CacheIdx },

/// `C::m(args)` / `self::` / `parent::` / `static::` / `$cls::m(args)`.
/// Forwarding rules for LSB in §4; honors __callStatic.
CallStatic { dst: Reg, class: ClassRef, name: NIdx, base: Reg, argc: u16, cache: CacheIdx },

// -- type test / clone --
/// `$v instanceof C`. False for non-objects; membership in the class's `ancestors`.
InstanceOf { dst: Reg, val: Reg, class: ClassRef },

/// `clone $o`. Shallow property copy + new id + new Rc, then run __clone.
Clone { dst: Reg, src: Reg },

// -- lvalue cursor additions (see §3) --
/// Begin a cursor whose descent step into a property; used after LvalCv/LvalProp.
LvalProp { name: NIdx },
/// Begin a cursor ROOTED at a static property slot (for `C::$p = v`, `C::$p[$i]=v`).
LvalStaticProp { class: ClassRef, name: NIdx },
```

`?` (may enter the unwinder) applies to `New`, `FetchProp`, `FetchStaticProp`,
`FetchClassConst`, `CallMethod`, `CallStatic`, `Clone` (any can raise an `Error`:
abstract instantiation, undefined method/const, call-on-null). `InstanceOf` never
throws.

**No `AssignProp` opcode.** Property writes go through the cursor (`AssignLval`),
so compound assign / inc-dec / nested `$o->a->b[$i]` reuse one code path (§3) —
this mirrors the decision that arrays have no direct element-store opcode.

### 2.3 Side tables touched

- `names` (existing): method names, property names, class names (via
  `ClassRef::Name`), constant names — all interned `NIdx`, lowercased where PHP is
  case-insensitive (class/method/const names; **property names are
  case-sensitive**, so property `NIdx`s keep original case).
- `caches` (existing `Vec<InlineCache>`): one slot per IC-bearing site (§5).
- `Program.classes` (new): the `ClassProto` pool.

No new owned data in `Instr`; the enum stays `Copy`.

---

## 3. Property lvalues — extending the cursor, not forking it

The VM already holds a cursor as `cursor_root: Reg` + `cursor_steps:
Vec<CursorStep>` where `CursorStep ∈ {Key(Value), Append}` (vm `lib.rs`). Objects
slot into this with **one new step** and **one new root kind**, and the existing
`read_path`/`assign_path`/`unset_path` recursion gains one arm each.

### 3.1 CursorStep + CursorRoot additions

```rust
enum CursorStep {
    Key(Value),
    Append,
    Prop(PhpStr),          // NEW: descend into object property `name` (case-sensitive)
}

enum CursorRoot {          // NEW: the cursor may root at a CV or a static-prop cell
    Cv(Reg),               // existing behavior (LvalCv)
    Cell(Rc<RefCell<Value>>),  // a static property slot (LvalStaticProp)
}
```

`Vm.cursor_root: Reg` becomes `Vm.cursor_root: CursorRoot`. `LvalCv { cv }` sets
`Cv(cv)`; `LvalStaticProp { class, name }` resolves the class, grabs the static
slot's `Rc<RefCell<Value>>`, and sets `Cell(rc)`. `AssignLval`/`FetchLvalRw`/
`UnsetLval`/`IncDecLval` are **unchanged in shape** — they just dispatch on the
root: `Cv(r)` → operate on `&mut regs[r]` as today; `Cell(rc)` → operate on
`&mut *rc.borrow_mut()`.

### 3.2 The path recursion gains one arm (descend through the handle)

Objects are `Rc<RefCell<ObjectData>>`, so a `Prop` step descends **through the
RefCell to the property slot** — and because it is a handle, there is *no COW*: a
write through any alias mutates the shared object (contrast arrays, which
`make_mut`). The new arm in `assign_path` (and the mirror in `read_path`/
`unset_path`):

```rust
// in assign_path(slot: &mut Value, steps, value)
match (slot, head) {
    // ... existing (Array, Key) / (Array, Append) / (Ref, _) arms ...
    (Value::Object(handle), CursorStep::Prop(name)) => {
        let mut od = handle.borrow_mut();                 // descend through the handle
        let child = od.properties.entry(name.clone())
                      .or_insert(Value::Null);            // autovivify a declared-or-dynamic prop slot
        assign_path(child, rest, value)                   // reuse the SAME recursion for the tail
    }
    (Value::Null, CursorStep::Prop(_)) =>
        Err(Thrown::error("Error", "Attempt to assign property on null")),
    (scalar, CursorStep::Prop(_)) =>                       // "Creation of dynamic property on a scalar"
        Err(Thrown::error("Error", "Attempt to assign property on <type>")),
}
```

Key points:
- **A property value that is itself an array** descends further with the existing
  `(Array, Key)` arm — so `$o->p[$i] = v` autovivifies `p` to `[]` (if null) and
  then does an ordinary COW array write *inside the object's property slot*. The
  object is a handle, the property is a value: both disciplines compose, each at
  its own level.
- **`&`-bound properties** are `Value::Ref` property values, caught by the
  existing `(Ref, _)` arm — the §4 reference machinery is reused unchanged
  (`value-model.md` §5).
- A `Cell` root for `C::$p[$i] = v` borrows the static cell mut, then the same
  recursion runs; no object-specific code.

### 3.3 Compiler side: `collect_lvalue_steps` grows two cases

`collect_lvalue_steps` (compiler `lib.rs`) already walks `ArrayAccess`/
`ArrayAppend` down to a root CV, pushing `CursorStep`s. It gains:

```rust
Expression::Access(Access::Property(pa)) => {           // $o->p ...
    let root = self.collect_lvalue_steps(pa.object, steps)?;   // descend to the base
    let name = self.intern_prop_name(&pa.property)?;           // ClassLikeMemberSelector::Identifier
    steps.push(CursorStep::Prop(name));
    Ok(root)
}
Expression::Access(Access::StaticProperty(sp)) => {     // C::$p is a cursor ROOT, not a step
    // emitted specially: the cursor begins with LvalStaticProp instead of LvalCv
    // (see emit_cursor change below) — returns a sentinel the emitter recognizes.
}
```

`emit_cursor` begins with `LvalCv { cv: root }` for a CV root, or
`LvalStaticProp { class, name }` for a static-prop root; the `CursorStep::Prop`s
emit `LvalProp { name }`. Everything after (`AssignLval`/`FetchLvalRw`/
`IncDecLval`) is byte-for-byte the existing compound-assign/inc-dec code in
`compile_assignment`/`compile_inc_dec`.

---

## 4. Late static binding (threaded through the frame)

`Frame` gains two `Option<ClassId>` fields (and keeps `$this` in its CV):

```rust
struct Frame {
    proto: Rc<FunctionProto>,
    regs: Vec<Value>,
    pc: usize,
    ret_dst: Reg,
    called_class: Option<ClassId>,    // LSB: resolves `static::`   (runtime class the call entered on)
    defining_class: Option<ClassId>,  // resolves `self::` / `parent::` (where the body is declared)
}
```

- **`self::`** → `defining_class` (resolve method/const/prop on that class).
- **`parent::`** → `classes[defining_class].parent`.
- **`static::`** → `called_class` (the *runtime* class — late static binding).

How the two are set when a frame is pushed, verified against `php` 8.5.8:

| Call form | `called_class` of new frame | `defining_class` of new frame | $this |
|---|---|---|---|
| `new C()` / `new static()` | resolved `C` / caller's `called_class` | ctor's `defining_class` | the new object |
| `$o->m()` (`CallMethod`) | **`$o`'s runtime class** | `m`'s `defining_class` | `$o` |
| `C::m()` (non-forwarding) | resolved `C` | `m`'s `defining_class` | forwarded `$this` if compatible, else none |
| `$cls::m()` / `$o::m()` (non-forwarding) | resolved runtime class | `m`'s `defining_class` | — |
| `self::m()` / `static::m()` / `parent::m()` (**forwarding**) | **caller's `called_class`** (preserved) | `m`'s `defining_class` | forwarded `$this` |

The rule in one line: **method calls on an instance, and `ClassName::`/`$x::`
static calls, SET the called-class; `self::`/`parent::`/`static::` calls FORWARD
(keep) the caller's called-class.** Probed:

```
class A { static function create(){ return static::class; }
          static function viaName(){ return A::create(); }       // non-forwarding
          static function viaStatic(){ return static::create(); }// forwarding
          static function viaSelf(){ return self::create(); } }   // forwarding
class B extends A {}
B::create()   => "B"   B::viaName()   => "A"
B::viaStatic()=> "B"   B::viaSelf()   => "B"
// parent:: keeps static:: too:
class A2 { function id(){ return static::class; } }
class B2 extends A2 { function id(){ return "B+".parent::id(); } }
class C2 extends B2 {}
(new C2)->id() => "B+C2"      // parent::id runs A2's body, static:: still C2
```

So `CallStatic` computes the new frame's `called_class` as: `StaticClass`/
`SelfClass`/`ParentClass` → copy `self.frame.called_class`; `Name`/`Dynamic` →
the freshly resolved id. `CallMethod` always sets `called_class = object's class`.
`New` sets `called_class = resolved class` (so `static::` inside a ctor is the
instantiated class). `{main}` and free functions carry `None` for both; `static::`
there is a compile-time error, matching PHP.

---

## 5. Inline caches (reserved, `ClassId`-guarded, monomorphic)

Per `bytecode.md` §2.5, each IC-bearing opcode carries `cache: CacheIdx` into the
proto's `Vec<InlineCache>` side table. `InlineCache { class_id: u32, payload: u32
}` is `Cell`-based (single-threaded, no atomics). The guard-and-fill shape, same
for every site:

```rust
// FetchProp fast path
let handle = /* deref obj */;
let cls = handle.borrow().class;
let ic = &proto.caches[cache as usize];
let slot_idx = if ic.class_id.get() == cls + 1 {       // +1 so 0 == "empty IC"
    ic.payload.get()                                   // HIT: cached property slot index
} else {
    let idx = self.resolve_prop_slot(cls, name)?;      // MISS: ordinary resolver
    ic.class_id.set(cls + 1); ic.payload.set(idx);     // fill
    idx
};
```

- **`FetchProp`/`LvalProp`**: `payload` = the property's index in the class's
  `prop_template` ordering, so a hit is an `IndexMap::get_index` (array-ish) with
  no name hash.
- **`CallMethod`/`CallStatic`**: `payload` = an index into a resolved-method side
  vector (`Rc<FunctionProto>` + `defining_class` + `is_static`), so a hit skips
  the `methods` hash probe.
- **`New`/`FetchStaticProp`/`FetchClassConst`**: `payload` = the resolved
  `ClassId` for a `ClassRef::Name` (the name→id probe is the cost avoided);
  `self`/`parent`/`static`/`Dynamic` never cache (they depend on the frame or a
  runtime value — guard would always miss, so skip the IC for them).

Miss always calls the exact same resolver the cold path uses, so the IC is a pure
speedup with no separate semantics — the JIT lowers the guard inline and calls the
resolver on miss.

---

## 6. VM semantics per opcode (delegation points)

- **`New`**: `let id = resolve_class(class)?`; reject if
  `kind != Class || flags.abstract` → `Error "Cannot instantiate
  abstract class/interface C"`. Allocate `ObjectData { class: id, id:
  self.next_object_id (post-increment), properties: entry.prop_template.clone() }`,
  wrap `Rc::new(RefCell::new(..))`, write the handle to `dst` **first**. Then, if
  the class has `__construct`, `enter_user_call` on it with the `[base,
  base+argc)` window, `$this = handle`, `called_class = id`,
  `defining_class = ctor.defining_class`, return value discarded. No `__construct`
  with `argc > 0` is allowed only when args are explicitly empty — PHP ignores
  surplus args when there is no user ctor, so we ignore the window. (A throw in
  the ctor unwinds normally; `dst` already holds the orphaned handle, which PHP
  also constructs-then-discards.)
- **`FetchProp`**: deref `obj`. Non-object → warning `Attempt to read property
  "p" on <type>` + `Null`. Object: `properties.get(name)` → clone out; absent →
  if class has `__get`, call it with the name string (normal call path) and use
  its return; else warning `Undefined property C::$p` + `Null`.
- **`FetchStaticProp`**: `resolve_class`, `static_props.get(name)` →
  `rc.borrow().clone()`; absent → `Error "Access to undeclared static property"`.
- **`FetchClassConst`**: `resolve_class`; `name == "class"` → the class's name as
  `Value::Str` (works for `static::class`, `$o::class`); else
  `constants.get(name)` or `Error "Undefined constant C::K"`.
- **`CallMethod`**: deref `obj`; non-object → `Error "Call to a member function
  m() on <type>"`. Resolve `m` on the object's class; absent → `__call` (args
  packed into an array) or `Error`. Push frame (`$this = handle`,
  `called_class = object's class`, `defining_class = method.defining_class`). A
  static method invoked via `->` still binds nothing special; visibility checks
  deferred (baseline: all public).
- **`CallStatic`**: `resolve_class`; resolve `m`; absent → `__callStatic` or
  `Error`. `called_class` per §4 forwarding table; `$this` forwarded when the
  current `$this instanceof` the target and the method is non-static (the one
  deep corner; baseline may bind `$this = None` for static methods and revisit).
- **`InstanceOf`**: deref `val`; non-object → `Bool(false)`.
  `class` may be `Dynamic` naming a string/object. `Bool(resolve_class(class)
  .map(|c| entry(val.class).ancestors.contains(&c)).unwrap_or(false))`.
- **`Clone`**: deref `src` → handle; `let od = src.borrow();` build
  `ObjectData { class: od.class, id: next_object_id++, properties:
  od.properties.clone() }` (shallow — nested arrays/objects follow their own
  copy/handle discipline, so a cloned object *shares* nested object handles, per
  PHP shallow clone). Wrap new `Rc`, write to `dst`, then run `__clone` with
  `$this = new handle` if defined.

---

## 7. Lowering examples (AST → bytecode)

`tN` = temp, `cv(x)` = the CV for `$x`. New opcodes in **bold** context.

### 7.1 Instantiation + construct (incl. dynamic `new $cls`)
```php
$o = new Counter(5);
```
```
LoadInt  t0, 5
New      t1, Name(Counter), base=t0, argc=1      ; t1 = handle; __construct(5) runs with $this=t1
Assign   cv(o), t1
```
```php
$o = new $cls();                 // dynamic
```
```
New      t0, Dynamic(cv(cls)), base=t0, argc=0   ; resolve $cls (string|object) -> ClassId
Assign   cv(o), t0
```

### 7.2 Property read + null-safe read
```php
echo $o->name;
```
```
FetchProp t0, obj=cv(o), name("name"), cache=#0
Echo      t0
```
```php
echo $o?->addr?->city;           // null-safe chain short-circuits whole chain
```
```
Mov        t0, cv(o)
JmpNull    t0, Lnull             ; $o === null -> result null
FetchProp  t1, obj=t0, name("addr"), cache=#1
JmpNull    t1, Lnull
FetchProp  t0, obj=t1, name("city"), cache=#2
Jmp        Lend
Lnull: LoadNull t0
Lend:  Echo t0
```

### 7.3 Property lvalues through the shared cursor
```php
$o->p = 1;                       // simple property store
```
```
LoadInt     t0, 1
LvalCv      cv(o)                ; root = Cv(cv(o))
LvalProp    name("p")            ; step Prop("p")  -> descends through the handle
AssignLval  t0                   ; write (no COW; handle is shared)
```
```php
$o->items[$i] += 2;              // property -> array element, compound
```
```
<$i -> t0>
LvalCv      cv(o)
LvalProp    name("items")        ; Prop("items") (autovivify [] if null)
LvalKey     t0                   ; Key($i)  — ordinary array descent inside the prop
FetchLvalRw t1                   ; read current element (cursor stays open)
Add         t2, t1, <k 2>
AssignLval  t2
```
```php
$this->count++;                  // inc/dec on a property
```
```
LvalCv      cv($this)
LvalProp    name("count")
IncDecLval  t0, pre=false, delta=+1
```

### 7.4 Method call + null-safe method
```php
$sum = $acc->add(3)->total();    // chained instance calls
```
```
LoadInt     t0, 3
CallMethod  t1, obj=cv(acc), name("add"), base=t0, argc=1, cache=#3   ; $this=$acc, called=class($acc)
CallMethod  t1, obj=t1,     name("total"), base=t1, argc=0, cache=#4
Assign      cv(sum), t1
```
```php
$x = $o?->run();                 // null-safe method: skip call if null
```
```
Mov         t0, cv(o)
JmpNull     t0, Lnull
CallMethod  t0, obj=t0, name("run"), base=t0, argc=0, cache=#5
Jmp         Lend
Lnull: LoadNull t0
Lend:  Assign cv(x), t0
```

### 7.5 Static call / LSB / class const / static prop
```php
Logger::write("hi");             // non-forwarding: called = Logger
static::make();                  // forwarding: called = caller's called_class
echo Config::VERSION;            // class constant
Config::$count += 1;             // static property as lvalue
echo $x instanceof Throwable;
```
```
LoadConst   t0, "hi"
CallStatic  t1, Name(Logger),  name("write"), base=t0, argc=1, cache=#6

CallStatic  t2, StaticClass,   name("make"),  base=t2, argc=0, cache=#7   ; called_class forwarded

FetchClassConst t3, Name(Config), name("VERSION"), cache=#8
Echo        t3

LoadInt         t4, 1
LvalStaticProp  Name(Config), name("count")       ; cursor root = Cell(static slot)
FetchLvalRw     t5
Add             t6, t5, t4
AssignLval      t6

InstanceOf  t7, val=cv(x), Name(Throwable)
Echo        t7
```

### 7.6 Clone
```php
$copy = clone $obj;              // shallow copy + __clone
```
```
Clone   t0, src=cv(obj)          ; new id, cloned prop map, __clone($this=t0) runs
Assign  cv(copy), t0
```

---

## 8. Do-not-regret list (OOP-specific)

1. **Handle vs value at every cursor level.** A `Prop` step descends through
   `Rc<RefCell>` with **no `make_mut`** (shared mutation); a `Key`/`Append` step
   inside a property still COW-clones the array. Mixing these up turns objects
   into value types or arrays into handles. The composition is: object levels
   share, array levels COW, each caught by its own `assign_path` arm.
2. **Called-class vs defining-class.** `static::` reads `called_class`;
   `self::`/`parent::` read `defining_class`. `CallMethod` sets `called_class` to
   the object's *runtime* class; `self::`/`parent::`/`static::` calls *forward*
   it. Get the forwarding table (§4) wrong and every LSB fixture diverges.
3. **`prop_template` is cloned per instance, never shared.** `New` must
   `.clone()` the template map so two objects of one class don't alias properties;
   the clone is cheap (`Rc` bumps) but must happen.
4. **Static props are one shared cell per class.** Inherited-but-not-redeclared
   static props alias the ancestor's `Rc<RefCell<Value>>`; redeclaration makes a
   fresh cell. The cursor roots at the cell so `C::$s[$i] = v` writes the shared
   slot.
5. **Object id is monotonic and per-`clone`/`New`.** `===` is `Rc::ptr_eq` (so a
   fresh `Rc` per object is what distinguishes identity); the `id` is for
   `spl_object_id`/hashing and must advance on every `New` and `Clone`.
6. **Magic methods go through the normal call path.** `__construct`, `__get`,
   `__call`, `__callStatic`, `__clone` push ordinary frames with `$this` + LSB
   set — no special reentrancy, so a throw inside them unwinds by the §3.4 table
   like any call.
7. **Cycles leak (for now).** `$o->self = $o` is an `Rc` cycle; deferred cycle
   collector (`value-model.md` §0) cleans it later. Do not add a borrow that
   would deadlock on a self-referential property during cursor descent — distinct
   `RefCell`s are fine, a true self-cycle in one `borrow_mut` chain panics
   (acceptable baseline).
8. **ICs guard on `ClassId` only, store `+1`.** A zeroed `InlineCache` means
   empty; store `class_id + 1` so `ClassId 0` isn't mistaken for empty. Never
   cache `self`/`parent`/`static`/`Dynamic` class refs.
```
