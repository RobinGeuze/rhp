# OOP AST surface (mago) — compile catalog for phase 7

Scope: the complete `mago-syntax-1.58.2` CST surface the compiler must lower for
OOP. Source: `.../mago-syntax-1.58.2/src/cst/cst/` (arena-allocated; `'arena`
elided below). Exact Rust variant/field names are authoritative; each row has a
one-line interpreter note. OOP is **VM-only** (the oracle is frozen and skips
OOP fixtures). ISA framing: the OOP opcodes (`New`, `FetchProp`/`AssignProp`,
`CallMethod`, `CallStatic`, `InstanceOf`, `Clone`, late static binding) are
"same encoding and calling convention" as the rest of the ISA
(`bytecode.md` §OOP-deferred). `Program` already reserves `pub classes: Vec<PhpStr>`
and `Value::Object` carries a `ClassId(u32)` indexing a class table **in the `Vm`**.

Verified against local `php 8.5.8` (handle identity, clone, promotion,
enum-backed values, `->(...)` FCC, `instanceof`).

---

## 1. Declarations (top-level `Statement` variants)

`Statement::{Class, Interface, Trait, Enum}` (`cst/statement.rs`) — declarations are
statements. They are **hoisted**: register the class/interface/trait/enum in the Vm
class table before executing top-level code (like function declarations today).
Anonymous classes are expressions (§6), not statements.

### 1.1 `Class` (`class_like/mod.rs`)
```
struct Class {
    attribute_lists: Sequence<AttributeList>,
    modifiers:       Sequence<Modifier>,      // abstract / final / readonly
    class:           Keyword,
    name:            LocalIdentifier,
    extends:         Option<Extends>,         // single parent (types has 1)
    implements:      Option<Implements>,      // 0..n interfaces
    left_brace:      Span,
    members:         Sequence<ClassLikeMember>,
    right_brace:     Span,
}
```
Note: build a `ClassEntry` — assign a `ClassId`, record parent (`extends`), the
interface set (`implements`), and compile members into property/const/method
tables. `final`/`abstract`/`readonly` are enforcement flags (abstract class ⇒
`new` throws `Error`; readonly class ⇒ every property readonly).

### 1.2 `Interface`
```
struct Interface {
    attribute_lists, interface: Keyword, name: LocalIdentifier,
    extends: Option<Extends>,   // interfaces: `extends` lists MULTIPLE parents
    left_brace, members: Sequence<ClassLikeMember>, right_brace,
}
```
Note: no `implements` field; interface inheritance uses `extends` with N types.
Members are method signatures (abstract bodies) + `const`. No instantiation.

### 1.3 `Trait`
```
struct Trait { attribute_lists, r#trait: Keyword, name: LocalIdentifier,
               left_brace, members: Sequence<ClassLikeMember>, right_brace }
```
Note: no inheritance clause. Compile members, then **copy** them into each using
class at `use` resolution time (§5.1). Not instantiable.

### 1.4 `Enum`
```
struct Enum {
    attribute_lists, r#enum: Keyword, name: LocalIdentifier,
    backing_type_hint: Option<EnumBackingTypeHint>,   // `: int` / `: string`
    implements:        Option<Implements>,
    left_brace, members: Sequence<ClassLikeMember>, right_brace,
}
struct EnumBackingTypeHint { colon: Span, hint: Hint }
```
Note: an enum is a special class whose `EnumCase` members become singleton
instances (one `ObjectHandle` per case, created at registration, shared/`===`).
Backed enums carry `->value` (int/string per `backing_type_hint`) and expose
`::cases()`, `::from()`, `::tryFrom()`; every case also has `->name`. All cases
implement `UnitEnum`; backed also `BackedEnum`. Cases are **not** cloneable.

### 1.5 `Extends` / `Implements` (`class_like/inheritance.rs`)
```
struct Extends    { extends:    Keyword, types: TokenSeparatedSequence<Identifier> }
struct Implements { implements: Keyword, types: TokenSeparatedSequence<Identifier> }
```
Note: resolve each `Identifier` (Local/Qualified/FullyQualified) to a `ClassId`
via the class table + namespace rules. Class `extends` = 1 type; interface
`extends` and all `implements` = N types.

---

## 2. `Modifier` (`cst/modifier.rs`)

```
enum Modifier {
    Static(Keyword), Final(Keyword), Abstract(Keyword), Readonly(Keyword),
    Public(Keyword), PublicSet(Keyword),
    Protected(Keyword), ProtectedSet(Keyword),
    Private(Keyword), PrivateSet(Keyword),
}
```
Helpers: `ModifierSequenceExt` (`get_static`, `contains_abstract`,
`get_first_visibility`, `get_first_write_visibility`, …) and per-variant
`is_*`. `*Set` variants are **asymmetric visibility** (`public private(set)`,
PHP 8.4). `is_read_visibility` = Public/Protected/Private;
`is_write_visibility` = `*Set`.
Note: used on classes (abstract/final/readonly), methods, properties, class
constants (final + visibility), promoted params, and trait-use aliases. For the
first cut compile the enforcement flags; visibility checks can be a later layer.

---

## 3. `ClassLikeMember` (`class_like/member.rs`)

```
enum ClassLikeMember {
    TraitUse(TraitUse),            // §5.1
    Constant(ClassLikeConstant),   // §4.3
    Property(Property),            // §4.2
    EnumCase(EnumCase),            // §4.4
    Method(Method),                // §4.1
}
```
`ClassLikeMemberSequenceExt` gives `contains_methods()` etc. Iterate `members`
and dispatch per variant into the class entry's tables.

### 3.1 Member/constant selectors (shared by access + calls)
```
enum ClassLikeMemberSelector {           // property name / method name after -> ::
    Identifier(LocalIdentifier),         // ->foo / ->bar()         (static name)
    Variable(Variable),                  // ->$name / ->$name()     (dynamic)
    Expression(ClassLikeMemberExpressionSelector), // ->{expr}
    Missing(Span),                       // parse error / recovery
}
enum ClassLikeConstantSelector {         // after :: for a class constant
    Identifier(LocalIdentifier), Expression(ClassLikeMemberExpressionSelector), Missing(Span),
}
struct ClassLikeMemberExpressionSelector { left_brace, expression: &Expression, right_brace }
```
Note: `Identifier` ⇒ static member name (intern once, cache slot). `Variable` /
`Expression` ⇒ evaluate to a string at runtime, then dynamic lookup (no inline
cache). `Missing` ⇒ propagate the parse error. The `::class` pseudo-constant
appears as `ClassConstantSelector::Identifier("class")` — resolve to the FQCN
string, not a real constant.

---

## 4. Member kinds

### 4.1 `Method` (`class_like/method.rs`)
```
struct Method {
    attribute_lists, modifiers: Sequence<Modifier>,   // public/private/protected/static/abstract/final
    function: Keyword,
    ampersand: Option<Span>,                          // by-reference return `function &foo()`
    name: LocalIdentifier,
    parameter_list: FunctionLikeParameterList,         // §4.1.1 (promotion lives here)
    return_type_hint: Option<FunctionLikeReturnTypeHint>,
    body: MethodBody,
}
enum MethodBody { Abstract(MethodAbstractBody { terminator }), Concrete(Block) }
```
Helpers: `Method::is_abstract()`, `is_static()`, `has_promoted_properties()`.
Note: compile each `Concrete` body to a `FunctionProto` (same compiler path as
free functions) keyed by `(ClassId, lowercased name)`; prepend an implicit
`$this` binding for instance methods (name `b"$this"`, §7). `Abstract` bodies
emit no proto — just a signature slot (interface/abstract class). `static`
methods get no `$this`. Method names are **case-insensitive**. `__construct`,
`__destruct`, `__clone`, `__toString`, `__get`/`__set`/`__call`… are magic — at
minimum wire `__construct` (called by `New`, §6) and `__clone` (by `Clone`, §6.3).
Defer by-ref return (`ampersand`) until the reference phase.

#### 4.1.1 `FunctionLikeParameter` + constructor promotion (`function_like/parameter.rs`)
```
struct FunctionLikeParameter {
    attribute_lists, modifiers: Sequence<Modifier>,   // NON-EMPTY ⇒ promoted property
    hint: Option<Hint>,
    ampersand: Option<Span>,                          // by-ref param
    ellipsis: Option<Span>,                           // variadic ...$x
    variable: DirectVariable,
    default_value: Option<FunctionLikeParameterDefaultValue { equals, value: &Expression }>,
    hooks: Option<PropertyHookList>,                  // promoted + hooks (defer, §8)
}
// is_promoted_property() == !modifiers.is_empty() || hooks.is_some()
```
Note: in `__construct`, a param with any `modifiers` (or `hooks`) is **constructor
property promotion** — declare a property of that name/visibility/type on the
class AND bind the incoming arg to it, i.e. implicit `$this->x = $x;` on entry.
Non-promoted params behave exactly as free-function params (already implemented:
`bind_params`). Readonly-promoted ⇒ readonly property.

### 4.2 `Property` (`class_like/property.rs`)
```
enum Property { Plain(PlainProperty), Hooked(HookedProperty) }

struct PlainProperty {
    attribute_lists, modifiers: Sequence<Modifier>,
    var:  Option<Keyword>,                 // legacy `var $x;` keyword
    hint: Option<Hint>,                    // typed property
    items: TokenSeparatedSequence<PropertyItem>,   // `$a = 1, $b;`
    terminator,
}
struct HookedProperty { attribute_lists, modifiers, var, hint, item: PropertyItem, hook_list: PropertyHookList }

enum PropertyItem {
    Abstract(PropertyAbstractItem { variable: DirectVariable }),           // no default
    Concrete(PropertyConcreteItem { variable: DirectVariable, equals, value: &Expression }),  // = default
}
```
Helpers: `Property::modifiers()`, `hint()`, `variables()`, `first_variable()`.
Note: for each `PropertyItem`, record a property slot (name = `variable.name`
minus `$`, §7) in declaration order with its visibility/`static`/`readonly`
flags. `Concrete` defaults are **constant expressions** evaluated at class
registration to seed the instance's `properties: IndexMap` (static props seed a
per-class slot). `static` properties live on the class entry, not the instance.
A `PlainProperty` may declare several names in one statement (`items`). Handle
`Hooked`/`hook_list` → defer (§8).

### 4.3 `ClassLikeConstant` (`class_like/constant.rs`)
```
struct ClassLikeConstant {
    attribute_lists, modifiers: Sequence<Modifier>,    // final + visibility (8.1+)
    r#const: Keyword,
    hint: Option<Hint>,                                // typed const (8.3+)
    items: TokenSeparatedSequence<ClassLikeConstantItem>,
    terminator,
}
struct ClassLikeConstantItem { name: LocalIdentifier, equals: Span, value: &Expression }
```
Note: evaluate each `value` (constant expression; may reference `self::OTHER`,
enum cases, `::class`) and store on the class entry under `name` (case-SENSITIVE,
unlike method/function names). One statement can define multiple `items`.
Reachable via `ClassConstantAccess` (§6.2).

### 4.4 `EnumCase` (`class_like/enum_case.rs`)
```
struct EnumCase { attribute_lists, case: Keyword, item: EnumCaseItem, terminator }
enum EnumCaseItem {
    Unit(EnumCaseUnitItem   { name: LocalIdentifier }),                      // pure enum
    Backed(EnumCaseBackedItem { name: LocalIdentifier, equals, value: &Expression }), // = 'h' / = 1
}
// EnumCaseItem::name() -> &LocalIdentifier
```
Note: at enum registration, create one shared singleton `ObjectHandle` per case,
set `->name` = `name`, and (Backed) `->value` = evaluated `value` (must match
`backing_type_hint`). Register under `EnumName::CaseName` for `::` access, and
index backed values for `::from`/`::tryFrom`. `Unit` vs `Backed` must be
consistent with whether the enum has a `backing_type_hint`.

---

## 5. Trait use (`class_like/trait_use.rs`)

### 5.1 `TraitUse`
```
struct TraitUse {
    r#use: Keyword,
    trait_names: TokenSeparatedSequence<Identifier>,   // `use A, B;`
    specification: TraitUseSpecification,
}
enum TraitUseSpecification {
    Abstract(TraitUseAbstractSpecification(Terminator)),               // `use A;`
    Concrete(TraitUseConcreteSpecification { left_brace, adaptations: Sequence<TraitUseAdaptation>, right_brace }),
}
```
Note (core): resolve each trait name to its `ClassEntry` and **flat-copy** its
methods/properties/constants into the using class (compile method bodies in the
using class's context so `self`/`$this` bind correctly). Conflicts without
adaptations are a fatal error. This is the only required trait machinery for the
first cut.

### 5.2 Adaptations (DEFER — §8)
```
enum TraitUseAdaptation { Precedence(TraitUsePrecedenceAdaptation), Alias(TraitUseAliasAdaptation) }
struct TraitUsePrecedenceAdaptation { method_reference: TraitUseAbsoluteMethodReference,
                                      insteadof: Keyword, trait_names: TokenSeparatedSequence<Identifier>, terminator }  // `A::f insteadof B;`
struct TraitUseAliasAdaptation { method_reference: TraitUseMethodReference,
                                 r#as: Keyword, modifier: Option<Modifier>, alias: Option<LocalIdentifier>, terminator } // `A::f as g;` / `f as protected;`
enum TraitUseMethodReference { Identifier(LocalIdentifier), Absolute(TraitUseAbsoluteMethodReference) }
struct TraitUseAbsoluteMethodReference { trait_name: Identifier, double_colon: Span, method_name: LocalIdentifier }
```
Note: `insteadof` (precedence) and `as` (alias / re-visibility) rewrite the
member set during copy. Low frequency — defer; on encounter, error "trait
adaptations not yet supported".

---

## 6. Instantiation, access, calls (expressions)

### 6.1 `Instantiation` — `Expression::Instantiation` (`cst/instantiation.rs`)
```
struct Instantiation { new: Keyword, class: &Expression, argument_list: Option<ArgumentList> }
```
Note: `class` is an **expression**, not a bare name — resolve it:
`Identifier` ⇒ class by name; `Self_`/`Static`/`Parent` (§6.6) ⇒ contextual
class; `Variable`/other ⇒ runtime string or object → `ClassId`. Allocate
`ObjectData { class, id: next_id(), properties: <seeded defaults> }`, wrap in
`Rc<RefCell<>>`, then call `__construct` with `argument_list` (same arg
marshalling as `compile_call`; `None` = no parens = no args). Result is the new
handle. `new` on abstract/interface/enum ⇒ `Error`.

### 6.2 `Access` — `Expression::Access` (`cst/access.rs`)
```
enum Access {
    Property(PropertyAccess             { object: &Expression, arrow: Span,               property: ClassLikeMemberSelector }), // $o->p
    NullSafeProperty(NullSafePropertyAccess { object, question_mark_arrow: Span,          property: ClassLikeMemberSelector }), // $o?->p
    StaticProperty(StaticPropertyAccess { class:  &Expression, double_colon: Span,        property: Variable }),                // C::$p
    ClassConstant(ClassConstantAccess   { class:  &Expression, double_colon: Span,        constant: ClassLikeConstantSelector }),// C::K / C::class
}
```
Note:
- `Property` — eval `object` to a handle, read `properties[name]`; on write (lvalue
  cursor) insert/update. Missing prop ⇒ `__get`/dynamic prop/warning. `object` can
  chain (`$a->b->c`). Maps to `FetchProp`/`AssignProp`.
- `NullSafeProperty` — if `object` is null, short-circuit the whole access chain to
  null (no error); else as `Property`. Needs a null-check + skip.
- `StaticProperty` — `property` is a `Variable` (`C::$x`); resolve `class`
  (name / `self`/`static`/`parent`) to its entry and read/write the static slot.
- `ClassConstant` — resolve `class` then look up `constant` on the entry;
  `::class` (selector `Identifier("class")`) is special → FQCN string.
  Also the form `EnumName::CaseName` lands here, resolving to the case singleton.

### 6.3 `Call` — `Expression::Call` (`cst/call.rs`)
```
enum Call { Function(FunctionCall), Method(MethodCall), NullSafeMethod(NullSafeMethodCall), StaticMethod(StaticMethodCall) }
struct MethodCall       { object: &Expression, arrow: Span,               method: ClassLikeMemberSelector, argument_list: ArgumentList }  // $o->m()
struct NullSafeMethodCall { object: &Expression, question_mark_arrow: Span, method: ClassLikeMemberSelector, argument_list: ArgumentList }// $o?->m()
struct StaticMethodCall { class:  &Expression, double_colon: Span,        method: ClassLikeMemberSelector, argument_list: ArgumentList }  // C::m()
```
(`Function` is the existing free-call path.) Note:
- `Method` — eval `object` → handle; resolve `method` (static name ⇒ cached
  vtable slot by `ClassId`; dynamic ⇒ string lookup) walking the inheritance
  chain; push a frame binding `$this` = object. Marshal args like `compile_call`.
  Unknown method ⇒ `__call`. Maps to `CallMethod`.
- `NullSafeMethod` — null `object` ⇒ whole expression is null, args NOT evaluated.
- `StaticMethod` — resolve `class` (name/`self`/`static`/`parent`); call without
  `$this` for a true static, but `parent::m()`/`self::m()` from an instance method
  **forward** the current `$this` and late-static-binding class. Maps to `CallStatic`.
First-class-callable form `$o->m(...)` / `C::m(...)` / `strlen(...)` parses as
`PartialApplication` (not `Call`) — see §8.

### 6.4 `Clone` — `Expression::Clone` (`cst/clone.rs`)
```
struct Clone { clone: Keyword, object: &Expression }
```
Note: eval `object` to a handle, make a **new** `ObjectData` with a fresh `id`,
same `class`, and a shallow-copied `properties` map (new `Rc`); then invoke
`__clone` on the copy if defined. This is the one place objects "copy"
(value-model §5). `===` of original vs clone is false (verified).

### 6.5 `instanceof` — `BinaryOperator::Instanceof` (`cst/binary.rs`)
```
enum BinaryOperator { …, Instanceof(Keyword), … }   // in Binary { lhs, operator, rhs }
```
Helpers: `BinaryOperator::is_instanceof()`; `Expression::evaluates_to_boolean()`
treats it as bool. Note: appears as an ordinary `Binary`; `lhs` is the value,
`rhs` is the class — usually an `Identifier`, but may be a `Variable`/expression
(dynamic class name or an object whose class is used). Eval `lhs`; if it is an
object, test its `ClassId` against `rhs`'s resolved class/interface **including
ancestors + implemented interfaces**; non-object ⇒ false. Maps to `InstanceOf`.

### 6.6 Class-reference keywords — `Expression::{Parent, Static, Self_}`
```
Expression::Parent(Keyword)   // parent
Expression::Static(Keyword)   // static   (late static binding; is_constant()==false)
Expression::Self_(Keyword)    // self
```
Note: these are class-position expressions used as the `class`/`rhs` of §6.1–6.5.
Resolve against the current method's context: `Self_` = the lexically enclosing
class (compile-time `ClassId`); `Parent` = its parent; `Static` = the **runtime**
class of `$this` / the LSB class (resolved dynamically, carried on the frame).
`self`/`parent` are compile-time; `static` needs the called-class on the frame.

---

## 7. `$this` and member names

- `$this` is a plain `Variable::Direct(DirectVariable { span, name })` with
  `name == b"$this"` (mago keeps the leading `$` in `name`; strip it for the
  var table — same rule as every variable, `ast-surface.md` §var). No dedicated
  node. Bind it as a normal CV in the method frame on entry (instance methods
  only); it is absent/illegal in static methods and free functions.
- Property/method identifiers (`LocalIdentifier.value`) and variable names
  carry raw bytes; method & class names are case-insensitive (lowercase before
  keying vtables), class constant & property names are case-sensitive.

---

## 8. Defer list (parse, then error "not yet supported" or ignore)

- **Property hooks** — `HookedProperty`, `PropertyHookList`, `PropertyHook`,
  `PropertyHookBody` (Abstract/Concrete→Block|Expression), and promoted-param
  `hooks`. get/set accessors (PHP 8.4). Defer; treat a hooked property as a
  plain backed property is unsafe, so error on hooks until implemented.
- **Asymmetric visibility** `*Set` modifiers (`PublicSet`/`ProtectedSet`/
  `PrivateSet`) — parse; enforcement deferred.
- **Trait adaptations** — `TraitUseAdaptation` (`insteadof` precedence, `as`
  alias/visibility). Core `use A, B;` copy is supported; adaptations deferred.
- **By-reference return** — `Method.ampersand` (and by-ref params in a method)
  wait for the reference phase.
- **First-class callables / partial application** — `Expression::PartialApplication`
  (`$o->m(...)`, `C::m(...)`), its `PartialApplication::{Function,Method,StaticMethod}`
  carrying `ClassLikeMemberSelector`. Separate node from `Call`; build `Closure`
  objects — defer past the first cut.
- **Enum methods / interfaces beyond cases** — enums may carry `Method` and
  `Constant` members and `implements`; the case-singleton + `cases/from/tryFrom`
  machinery is core, but user methods on enums can follow the generic method path
  once methods work. Enum `implements` interface conformance can be deferred.
- **Attributes** — `attribute_lists` on every node: parse and ignore at runtime
  (no reflection yet).
- **Magic methods beyond `__construct`/`__clone`** — `__get`/`__set`/`__call`/
  `__callStatic`/`__toString`/`__isset`/`__invoke` etc.: stub to default
  behavior (error/warning) first, fill in incrementally.
- **Anonymous classes** — `Expression::AnonymousClass` (`AnonymousClass` in
  `class_like/mod.rs`: `new class(args) extends/implements { members }`): same
  member surface as `Class` plus `argument_list: Option<PartialArgumentList>`;
  needs a synthesized one-off `ClassId`. Defer until named classes work.

---

## Summary

The OOP surface is four declaration forms (`Statement::{Class, Interface, Trait,
Enum}`, all hoisted into a Vm class table keyed by `ClassId`) whose bodies are a
`Sequence<ClassLikeMember>` of `{TraitUse, Constant, Property, EnumCase, Method}`,
with `Modifier` flags and constructor property promotion carried on
`FunctionLikeParameter.modifiers` — and a matching expression set
(`Instantiation`, `Access::{Property, NullSafeProperty, StaticProperty,
ClassConstant}`, `Call::{Method, NullSafeMethod, StaticMethod}`, `Clone`,
`BinaryOperator::Instanceof`, and the `Expression::{Self_, Parent, Static}`
class-reference keywords), where `$this` is just a `DirectVariable` named
`$this`. Objects stay handle types (`Rc<RefCell<ObjectData>>` carrying a `u32`
`ClassId`): assignment shares, `clone` shallow-copies with a fresh id + `__clone`,
`===` is `Rc::ptr_eq` — all consistent with `value-model.md` §5 and verified
against `php 8.5.8`. Defer property hooks, trait adaptations, asymmetric
visibility, by-ref return, first-class callables/partial application, anonymous
classes, attributes, and the non-`__construct`/`__clone` magic methods.

File: `/Users/robing/gits/phrust/docs/design/oop-ast.md`
