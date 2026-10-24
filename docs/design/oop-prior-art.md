# rhp OOP — prior art on object models + method dispatch

Status: **design input** for the OOP phase (bytecode.md "phase 7"). Target
semantics: PHP 8.5 (ground truth: local `php` 8.5.8, probed inline below). This
doc surveys how Zend (PHP's own engine) and V8 model objects and dispatch
methods, then distills a concrete, rhp-shaped first cut. It does not change
the ISA: OOP keeps "the same encoding and calling convention" promised in
[`bytecode.md`](./bytecode.md) §2.4 and reuses the reserved inline-cache side
table ([`bytecode.md`](./bytecode.md) §2.5) and the `ClassId` handle model fixed
in [`value-model.md`](./value-model.md) §5.

Non-negotiable invariants carried in from the rest of the design:

- Single-threaded `Rc`/`RefCell`; no `Arc`/`Mutex`, no atomics in caches.
- `Value` is owned `'static` and pointer-sized: an object carries a
  `class: ClassId(u32)`, **not** an `Rc<ClassEntry>`. The class table lives in
  the `Vm`, never in a `Value` ([`value-model.md`](./value-model.md) §5,
  [`bytecode.md`](./bytecode.md) §3.1).
- Objects are **handle** types (`ObjectHandle = Rc<RefCell<ObjectData>>`):
  assignment shares, `clone` shallow-copies + new id, `===` is `Rc::ptr_eq`.
- Opcodes stay thin wrappers that delegate to `ops::`/`conv::`/`registry()`;
  the ISA stays JIT-lowerable (fixed `Copy` operands, side tables, instruction
  -index PCs).

---

## (a) Zend's object model — what to borrow, what to simplify

### What Zend actually has

A class is a `zend_class_entry` (`Zend/zend.h`). The fields that matter here:

| `zend_class_entry` field | what it is | rhp analogue |
|---|---|---|
| `parent` / `parent_name` | pointer to the single parent CE | `parent: Option<ClassId>` |
| `function_table` | `HashTable` name → `zend_function` (methods) | flattened method table (see (d)) |
| `properties_info` | `HashTable` name → `zend_property_info` (slot **offset**, flags, visibility, type, defining scope) | `IndexMap<PhpStr, PropInfo>` with declaration index |
| `default_properties_table` / `default_properties_count` | the initial values for declared property slots, copied into each new object | `default_props: Vec<Value>` (by slot) |
| `constants_table` | class constants | `IndexMap<PhpStr, ClassConst>` |
| `default_static_members_table` / `static_members_table` | static properties (live storage, shared per class) | `statics: RefCell<Vec<Value>>` on the class entry |
| `interfaces` / `num_interfaces` | implemented interfaces (flattened, transitive) | `Vec<ClassId>` (flattened set) |
| `iterator_funcs_ptr`, `*_handler` slots, `create_object` | per-class behavior hooks (magic methods, internal classes) | deferred: a small `handlers`/magic-method index later |

An **object** is a `zend_object` (`Zend/zend_types.h`):
`{ zend_refcounted_h gc; uint32_t handle; zend_class_entry *ce; const
zend_object_handlers *handlers; HashTable *properties; zval
properties_table[1]; }`. The crucial detail is the split storage:

- **Declared properties live in `properties_table`**, a flat C array *inlined
  into the object allocation* (the `[1]` is a struct-hack; Zend over-allocates
  `sizeof(zend_object) + (ce->default_properties_count - 1) * sizeof(zval)`).
  A property access compiled against a known class is **one pointer-add**:
  `OBJ_PROP(obj, prop_info->offset)` where `offset` is a byte offset baked into
  `zend_property_info` at class-link time. No hashing on the hot path.
- **Dynamic properties** (`$o->undeclared = 1`) spill into the lazily-allocated
  `properties` `HashTable`, keyed by name. In PHP 8.2+ this is deprecated unless
  the class is `#[AllowDynamicProperties]`; probed on 8.5.8:
  ```
  class C { public int $x = 1; } $o = new C(); $o->y = 5;
  // Deprecated: Creation of dynamic property C::$y is deprecated
  ```
  So the common case is "all properties are declared and slotted"; the hash is
  the slow fallback.

**`$this`, scope, and late static binding.** Zend threads *two* class pointers
through every call via the execute-data (`zend_execute_data`):

- `EX(This)` — the `$this` zval (the receiver object, or the class for a static
  call made in an instance context).
- `called_scope` (a.k.a. `EX(This).u2.extra` via `Z_CE`) — the **late static
  binding** class: the class the call was *dispatched through*, used by
  `static::` and `new static`.
- `EX(func)->common.scope` — the **defining scope**: the class the running
  method was *declared in*, used by `self::`, `parent::`, and private-member
  visibility.

These three are distinct and all needed. Probed on 8.5.8:
```php
class A { static function who(){ return static::class; }
          static function t(){ return self::class."/".static::class; } }
class B extends A {}
echo B::who();  // "B"   — static:: = called_scope (B)
echo B::t();    // "A/B" — self:: = defining scope (A), static:: = called_scope (B)
```

**Bound `$this` in closures.** A closure is a real object of class `Closure`
wrapping a `zend_closure` that stores a bound `this_ptr` **and** a bound
`called_scope`/`func.common.scope`. `Closure::bind`/`bindTo` produce a *new*
closure with a different `$this` and scope (which is how a closure can read
another object's private properties). Probed:
```php
class C { private int $x = 7; }
$f = Closure::bind(fn() => $this->x, new C, C::class);
echo $f();  // 7  — bound $this + bound scope lets it see private $x
```

### What to borrow, what to simplify for rhp

**Borrow:**

1. **The `ClassId` → class-entry table in the `Vm`.** Mirrors `ce`, keeps
   `Value` `'static`. `ObjectData.class` is the `ClassId`; the entry is
   `vm.classes[class_id as usize]`.
2. **The declared-vs-dynamic split as a *concept*** (declared props are known at
   class-link time; dynamic props are the deprecated fallback). rhp gets the
   same two tiers even though the first cut stores both in one map (see (c)).
3. **Three class pointers on the frame.** Add to `Frame` (or the method-call
   convention): `this: Option<ObjectHandle>`, `called_scope: ClassId` (LSB,
   drives `static::`/`new static`), `defining_scope: ClassId` (drives
   `self::`/`parent::`/private visibility). This is the single most important
   thing to get right and is cheap — three small fields.
4. **Closures carry bound `this` + bound scope**, set at creation and rebindable
   by `Closure::bind`. Reuse the same three-pointer mechanism.

**Simplify / defer:**

- **No struct-hack inlined `properties_table`.** rhp objects are
  `Rc<RefCell<ObjectData>>` with an `IndexMap<PhpStr, Value>` already in the
  value model. The packed-offset optimization is a *later* internal change (see
  (c)) that needs no ISA change.
- **No `zend_object_handlers` vtable yet.** Magic methods (`__get`, `__set`,
  `__call`, `__invoke`, `__toString`, `__clone`) become a small
  `Option<MethodId>` lookup on the class entry when their phase lands, not a
  full handler-pointer table.
- **Static members as `RefCell<Vec<Value>>` on the class entry**, not a separate
  per-request copied table; single-threaded means no `CG`/`EG` split.
- **Readonly / asymmetric visibility (`private(set)`, 8.4) / property hooks
  (8.4) / typed-property coercion** are flags on `PropInfo` to be enforced
  incrementally; the storage shape does not change.

---

## (b) Method dispatch + inline caches

### How Zend and V8 do it

**Zend.** A method call opcode (`ZEND_INIT_METHOD_CALL`) must map
`(receiver class, method name)` → `zend_function*`. The cold path is
`zend_hash_find(&ce->function_table, lc_method_name)` (names are
case-insensitive, so the table is keyed on the lowercased name). Because that
hash lookup every call would be slow, Zend attaches a **run-time cache** to the
op array: `op_array->run_time_cache`, a flat `void*[]` slab indexed by a
compile-time **cache slot** (`CACHE_PTR(opline->result.num)` /
`Z_CACHE_SLOT`). For a method call the slot stores a **polymorphic 2-word pair**
`{ cached_ce, cached_fbc }` (class entry the lookup was for, and the resolved
function). Dispatch is:

```
if (EXPECTED(cache->ce == obj->ce)) { fbc = cache->fn; }   // hit: pointer compare
else { fbc = zend_hash_find(...); cache->ce = obj->ce; cache->fn = fbc; }  // miss: fill
```

The guard is a **single pointer (class-identity) compare** — this is a
*monomorphic inline cache keyed on the receiver's class*. Zend's run-time cache
is a **side slab** (not bytes interleaved in the instruction stream), one slot
per call *site*, which is exactly the shape rhp already reserved.

**V8.** The same idea, generalized. Every object points to a **Map** (a.k.a.
"hidden class") that describes its layout and type. A property/method access
site keeps an inline cache that starts **uninitialized → monomorphic**: it
caches `{ expected Map, handler }` where the handler is "load slot N" or "call
this function". On access it does `if (object.map == cachedMap) use handler`.
Repeated sites with one receiver shape stay *monomorphic* (one compare, then a
direct slot/func); sites that see 2–4 shapes go *polymorphic* (a tiny list of
`{Map, handler}`); more go *megamorphic* (fall back to the runtime hash). The
Map pointer plays exactly the role of Zend's `ce` and of rhp's `ClassId`.

The shared lesson: **a 1-entry cache keyed on the receiver's class/map, guarded
by one identity compare, with a side-table slot per call site, collapses the
"name → member" lookup to a compare + a direct index on the hot path.**

### Where rhp puts the cache (no ISA change)

The ISA already has the slots:

- `FunctionProto.caches: Vec<InlineCache>` with
  `InlineCache { class_id: u32, payload: u32 }` ([`bytecode.rs`] lines
  178–183). `class_id` is the guard key; `payload` is the resolved target
  (a `MethodId` / flattened method-table index, or a property **slot index**).
- `CacheIdx = u32` is a reserved operand type, and `CallUser`/`CallByName`
  already carry a `cache` field ([`bytecode.md`](./bytecode.md) §2.4, §2.5).

So the phase-7 OOP opcodes (spelled out as reserved in
[`bytecode.md`](./bytecode.md) §2.4: `New`, `FetchProp`/`AssignProp`,
`CallMethod`, `CallStatic`, `InstanceOf`, `Clone`) each carry a `cache: CacheIdx`
operand into that side table. Dispatch in the VM:

```rust
// CallMethod { dst, recv, name: NIdx, base, argc, cache }
let obj = self.reg_get(top, recv);          // Value::Object(h)
let cid = obj.borrow().class;               // ClassId (u32)
let slot = &proto.caches[cache as usize];
let method = if slot.class_id == cid && slot.class_id != NO_CLASS {
    slot.payload                            // HIT: one u32 compare
} else {
    let m = self.resolve_method(cid, name)?; // MISS: hash in the class entry
    // Cell-based fill (single-threaded; no atomics):
    proto.caches[cache as usize] = InlineCache { class_id: cid, payload: m };
    m
};
self.enter_method_call(method, obj, /*called_scope=*/cid, dst, base, argc)?;
```

Key properties, all already consistent with the design:

- **Monomorphic to start** ([`bytecode.md`](./bytecode.md) §2.5 says exactly
  this). One-entry, `ClassId`-guarded. Polymorphic (a small inline `[(ClassId,
  payload); 4]`) and megamorphic fallback are a later tuning step that does not
  touch the opcode shape — only the `InlineCache` struct and the miss handler.
- **`Cell`-based, no atomics.** Single-threaded, so the cache is interior
  -mutable in place. The `caches` `Vec` is per-proto; make it
  `Vec<Cell<InlineCache>>` (or take `&mut` via the frame's `Rc<FunctionProto>`
  carefully) so a hot loop that calls the same method fills once.
- **Miss path is the plain resolver** — the *same* code the cold dispatch and a
  future JIT deopt would call. This is the §2.3 delegation discipline applied to
  dispatch: the cache is pure acceleration, never a second source of semantics.
- **JIT-friendly** for the same reasons the rest of the ISA is: the guard is a
  fixed-width `u32` compare against a side-table slot at a statically known
  index; Cranelift lowers it as `load slot.class_id; icmp eq cid; br`. No
  mutable bytes in the code stream, no variable-length oplines.

A caution borrowed from Zend's bug history: the cache key must be the **receiver
class**, not the static call-site class, and must be invalidated conceptually if
classes can be (re)declared at runtime. rhp declares classes via a hoist
pre-pass / `DeclareClass` and does not redefine them, so a filled slot stays
valid for the process — simpler than Zend, which ties the run-time cache
lifetime to the request and a `map_ptr` base.

---

## (c) Property access — packed slots vs. name hash

### The two mechanisms

- **Zend (packed slots).** At class-link time every declared property gets a
  `zend_property_info.offset` — a byte offset into the object's inlined
  `properties_table`. `$o->x` compiled against a known layout is
  `*(zval*)((char*)obj + offset)`: no hashing. A dynamic/unknown-name access
  falls to `zend_std_read_property` → hash lookup in `ce->properties_info` then
  the object's `properties` `HashTable`. Property *inline caches* (same run-time
  -cache slab as methods) memoize `{ ce, offset }` so even
  `$this->x` in a loop is a guard + a pointer-add.
- **V8 (Map-described slots).** The Map says property `x` is "in-object slot 2"
  or "fast-properties backing-store index k"; the IC caches the Map + the slot
  handler. Adding a property *transitions* the object to a new Map
  (shape-transition tree). Same net effect: name resolved to an integer slot,
  guarded by a shape identity.

Both converge on: **resolve the property *name* to an integer *slot* once, cache
the (class, slot), and index directly thereafter.**

### Recommended first cut for rhp: name-keyed `IndexMap` now

[`value-model.md`](./value-model.md) §5 already fixes
`ObjectData.properties: IndexMap<PhpStr, Value, FxBuildHasher>` in declaration/
insertion order. For the first cut, **keep this and resolve properties by name**:

```rust
// FetchProp { dst, recv, name: NIdx, cache }
let h = /* Value::Object */;
let v = h.borrow().properties.get(&name_str).cloned().unwrap_or(Value::Null); // + __get later
```

Why this is the right pragmatic first cut:

- It is the storage the value model already commits to, and `IndexMap` preserves
  PHP's property *order* (which `var_dump`, `(array)` cast, `foreach ($o as ...)`,
  and serialization all observe).
- Declared and dynamic properties live in the *same* map, so dynamic properties
  (and the 8.5 deprecation notice) fall out for free — no second container.
- `FxBuildHasher` on a short interned `PhpStr` key is fast; correctness and
  oracle parity come first, and property access is not yet the proven
  bottleneck.

### How to move to packed offsets later **without an ISA change**

This is the load-bearing claim, so it is spelled out. The migration is purely
behind the opcode and the cache slot:

1. **At class link, assign each declared property a dense `slot: u32`** (0-based,
   in declaration order, parent slots first then child's — mirroring how Zend
   lays parent properties before child properties). Store this on the class
   entry: `props: IndexMap<PhpStr, PropInfo { slot, flags, default_idx, ... }>`.
2. **Change `ObjectData` storage to a hybrid**, same public handle type:
   ```rust
   pub struct ObjectData {
       pub class: ClassId,
       pub id: u32,
       pub slots: Vec<Value>,                 // declared props, indexed by slot
       pub dynamic: Option<IndexMap<PhpStr, Value, FxBuildHasher>>, // lazy spill
   }
   ```
   New objects copy `class.default_props` into `slots`; dynamic props allocate
   `dynamic` lazily (exactly Zend's `properties_table` + lazy `properties`
   split). Iteration order = declared slots in order, then dynamic insertion
   order.
3. **Fill the inline cache `payload` with the *slot index*** instead of leaving
   it unused: `FetchProp`/`AssignProp` with a `cache` hit become
   `obj.slots[slot].clone()` — one guard compare + one `Vec` index, Zend's
   `OBJ_PROP`. A miss (dynamic name, or a property not in this class's layout)
   hashes `class.props` then falls to `dynamic`.

Crucially: **the opcodes `FetchProp { dst, recv, name, cache }` /
`AssignProp { recv, name, src, cache }` are identical before and after.** The
operand set, the `NIdx` name, and the `CacheIdx` slot all stay; only the VM's
interpretation of `cache.payload` and the shape of `ObjectData` change. A
future JIT lowers the hit path to `load slots_ptr; load/store [slots_ptr +
slot*16]` with a class guard — the same monomorphic IC as methods.

Decision: **ship the name-keyed `IndexMap` first; keep `InlineCache.payload`
reserved so turning it into a slot index is an internal, ISA-preserving change.**

---

## (d) Inheritance table construction — flatten vs. walk; MRO with interfaces/traits

### The choice

- **Walk at dispatch:** store only the direct `parent`; on a method/constant/
  property miss, climb `parent` pointers until found. Minimal memory, but every
  miss is O(depth), and it interacts badly with inline caches (the cache would
  have to key on where the member was found, not just the receiver class).
- **Flatten at link:** when a class is declared, build its **complete** tables by
  copying the parent's flattened tables and then applying the child's own
  members (child overrides parent by name). Lookup is then always O(1) in the
  child's own table, which is precisely what the `ClassId`-keyed inline cache in
  (b) wants: a hit means "this exact class resolves this name to this target,"
  no walking.

**Recommendation: flatten at class-link time.** This is what Zend does
(`zend_do_inheritance` copies `parent->function_table` /
`default_properties_table` / `constants_table` into the child, then overlays the
child's members and fixes up property offsets). It makes the monomorphic IC
correct-by-construction and keeps the hot path walk-free. Memory cost is modest
(method/const tables are name→id maps of small entries; property slots are a flat
`Vec`), and rhp declares each class once per process.

Concretely, at `DeclareClass`:

1. Resolve `parent` (single class) and `implements` (interfaces). PHP requires
   the parent/interfaces to be already declared (the hoist pre-pass in
   [`ast-surface.md`](./ast-surface.md) §2 handles forward references for
   top-of-scope declarations; conditional declarations run `DeclareClass` at
   execution point).
2. **Methods:** start from a clone of `parent.methods` (name→`MethodId`), then
   insert this class's own methods, **overriding by (lowercased) name**. Record
   each method's `defining_scope = this ClassId` so `self::`/`private` resolve
   correctly even after inheritance.
3. **Properties:** copy parent's `props` (keeping their slots 0..k), append this
   class's new declared properties at slots k..n, and copy
   `parent.default_props` then extend with this class's defaults. (A redeclared
   property with the same name reuses the parent's slot, matching Zend.)
4. **Constants:** copy-then-override, same as methods.
5. **Interfaces:** compute the **transitive, flattened set** of implemented
   interface `ClassId`s (parent's interfaces ∪ this class's `implements` ∪ their
   parents). Store as a `Vec<ClassId>` (or a small set) so `instanceof` /
   `InstanceOf` is a membership test, not a walk.

### MRO concerns — and why PHP is *easier* than Python/C++

PHP has **single inheritance of implementation**, so there is no diamond problem
for methods and therefore **no C3 linearization / MRO to compute** (unlike
Python's `__mro__` or C++ virtual bases). The only linear chain is the `parent`
spine. The subtleties are:

- **Interfaces** add no implementation, only type identity and abstract
  signatures; "flatten the transitive interface set" (step 5) is all that is
  needed. Interfaces *can* multiple-inherit other interfaces (`interface C
  extends A, B`) — that only unions their constant/abstract-method sets and the
  type set; still no method-body ambiguity.
- **Traits** are the one place implementation is merged from multiple sources,
  and PHP resolves this **at compile/link time by flattening, not by a runtime
  MRO**: a trait's methods are copied into the using class as if written there
  (so `defining_scope` becomes the *using* class, and `self::`/`private` bind to
  it). A method defined in the class **wins over** a trait method of the same
  name; a method in **two used traits** with the same name is a **fatal
  conflict** unless resolved with `insteadof` / `as` aliasing
  (`TraitUse` adaptations, [`ast-surface.md`](./ast-surface.md) §10.2). So trait
  resolution is: expand traits into the class's own member set (applying
  `insteadof`/`as`), *then* run the normal parent-flatten of step 2 — traits are
  resolved **before** inheritance overlay. This keeps dispatch a flat table
  lookup; there is never a runtime trait walk.
- **Abstract methods / interface methods** contribute signatures to check (and
  an "is this class instantiable?" flag) but no dispatch entry; an abstract
  class with an unimplemented abstract method is an instantiation error.
- **`parent::foo()`** is a *static* dispatch to the parent's flattened method
  table using the **defining scope's** parent (not the receiver's), while keeping
  the original `$this` and `called_scope` — so it must bypass the receiver-class
  inline cache and resolve through `defining_scope.parent`. Worth a dedicated
  resolution path, not the monomorphic receiver IC.

Net: flatten everything at link time (traits → then parent overlay → then
interface-set union), key the inline caches on the receiver `ClassId`, and the
only non-receiver-cached dispatch forms are `self::`/`parent::`/`static::`
(scope-relative), which resolve against the frame's three class pointers from
(a).

---

## Summary of recommendations (the first cut)

| area | first cut | later (no ISA change) |
|---|---|---|
| class storage | `Vm.classes: Vec<ClassEntry>` keyed by `ClassId(u32)`; object holds `class: ClassId` | magic-method handler indices, readonly/visibility enforcement |
| scope | `Frame` carries `this`, `called_scope` (LSB), `defining_scope` | closures reuse the same three fields, rebindable via `Closure::bind` |
| method dispatch | monomorphic `ClassId`-guarded IC in `FunctionProto.caches` (`class_id` guard, `payload` = `MethodId`); miss → hash in flattened method table | polymorphic (≤4) then megamorphic fallback; JIT guard lowering |
| property access | name-keyed `IndexMap<PhpStr, Value>` (declared + dynamic together) | hybrid `slots: Vec<Value>` + lazy `dynamic`; `payload` = slot index; packed-offset `OBJ_PROP`-style access |
| inheritance | **flatten at `DeclareClass`**: traits → parent overlay → transitive interface set; O(1) lookup | — |

---

## Appendix — concrete rhp touch points (for the implementer)

- Uncomment/define `Program.classes` ([`bytecode.rs`] line 255 /
  [`bytecode.md`](./bytecode.md) §2.1) as `Vec<ClassEntry>`; add `DeclareClass`
  alongside the reserved `DeclareFn` ([`bytecode.md`](./bytecode.md) §2.4 misc).
- Add to the `Vm`: `classes: Vec<ClassEntry>`, `class_ids: HashMap<Vec<u8>,
  ClassId>` (the slots `bytecode.md` §3.1 already notes as "phase 7"). Seed from
  the program's class list during `install`.
- Extend `Frame` ([`vm.md`/§3.2]) with `this: Option<ObjectHandle>`,
  `called_scope: ClassId`, `defining_scope: ClassId`; `enter_method_call`
  mirrors `enter_user_call` ([`rhp-vm/src/lib.rs`] lines 477–499) but also
  binds those three and reserves CV0 for `$this` if the method is non-static.
- Reuse `InlineCache { class_id, payload }` ([`bytecode.rs`] lines 178–183) as
  the monomorphic cache for both `CallMethod` and `FetchProp`/`AssignProp`; add
  a `cache: CacheIdx` operand to each phase-7 OOP opcode exactly as
  `CallUser`/`CallByName` already carry `cache`.
- `instanceof` ([`ast-surface.md`](./ast-surface.md) §5.1 `Instanceof`) → membership
  test against the flattened `interfaces` set ∪ the `parent` spine of the
  receiver's `ClassId`.
- `clone` ([`ast-surface.md`](./ast-surface.md) §3 `Clone`) is the one object "copy":
  new `id`, new `Rc`, cloned property map/slots, then run `__clone` — exactly as
  [`value-model.md`](./value-model.md) §5 states.
- All semantics claims above are probed against `php` 8.5.8; the probes
  (LSB `B`/`A/B`, dynamic-property deprecation, `Closure::bind` private access)
  are reproduced inline in (a)/(b) and should become golden fixtures (VM-only;
  the frozen oracle skips OOP fixtures).
