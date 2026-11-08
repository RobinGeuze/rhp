# Object handlers and native payloads

Status: implemented (Phase L6). How an internal class gets php's
`zend_object_handlers` behaviour and keeps native state with its objects.

php gives every object a handler table, set by its class's `create_object`
and inherited by subclasses, and lets an extension wrap `zend_object` in its
own struct. rhp has the same two pieces:

- **the payload**: one slot of native state on `ObjectData`
  ([`rhp_values::Payload`]), and
- **the handler table**: [`rhp_builtins::zend::object_handlers::ObjectHandlers`],
  registered per class by the extension that owns it.

The engine looks the table up once per class, when it links the class: a
built-in class takes its own registration, any other class (a user subclass
included) its parent's. A class with no handlers costs one field test
(`ClassEntry::handlers` is `None`), and only where an object meets a
conversion, operator, comparison or clone; property access never looks.

## The payload

```rust
let h = object_handlers::object_of(&v)?;                  // the ObjectHandle
object_handlers::set_payload(&h, Rc::new(MyState { … })); // when creating the object
let st: Rc<MyState> = object_handlers::payload(&v)?;      // in a builtin or handler
h.borrow_mut().take_payload();                            // free early (`close()`)
```

- Typed by downcast: a `payload::<T>()` for another type is `None`, so an
  argument check is the lookup itself (`socket_arg`, `parser_arg`).
- It is dropped with the object's storage. Its `Drop` is php's `free_obj`:
  it runs once, when the last reference goes, with no host, so it must not
  call into PHP. Work that may belongs in `dtor_obj` (an XMLWriter
  flushing to a user stream, before `__destruct` as in php) or
  `free_obj_host` (a ZipArchive's close running its callbacks, after it).
- Get an `Rc<T>` and release the object borrow before calling the host or
  PHP; mutate through `Cell`/`RefCell` inside `T`. A builtin's clone keeps
  the state alive until it returns, as php's refcount would.
- Never a strong reference back to the object: keep a `Weak` (ext/xml's
  `index`). The cycle collector sees a payload's values only through the
  class's `get_gc` handler (gc.md); without one they count as outside
  references and keep a cycle alive. Values a payload
  holds for PHP (ext/xml's handlers and `xml_set_object` target) can still
  form a cycle through the object; ext/xml keeps a thread-local list of
  weak references to its parsers and drops those values at the next
  request. That list is the one exception to "no side tables".
- **Send**: `Payload` is `Rc<dyn Any>` because objects are `Rc` and stay on
  their VM's thread, and today's payloads are `!Send` (`Cell`/`RefCell`,
  raw libxml pointers). Moving objects between threads would make the
  alias `Arc<dyn Any + Send + Sync>` and change every payload type with it:
  `Mutex` or atomics instead of `Cell`/`RefCell`, a `Send` wrapper for each
  library handle whose library allows it, and no thread-local state such
  as ext/xml's list.
- The slot costs one pointer per object (objects stay in the same
  memory_limit size class); the state itself is a separate allocation.

## The handlers

```rust
static MY_HANDLERS: ObjectHandlers = ObjectHandlers {
    compare: Some(my_compare),
    cast_object: Some(my_cast),
    ..ObjectHandlers::STD
};
pub(crate) static CLASSES: &[ClassHandlers] = &[
    ClassHandlers { class: "MyClass", handlers: &MY_HANDLERS, create_object: true },
];
```

and add `my_ext::CLASSES` to `class_handlers()` in `ext/mod.rs`. The
classes of the engine itself (`Exception`, `Generator`, Reflection…) are in
`zend/object_handlers.rs`. `ObjectHandlers::UNCLONEABLE` and `ObjectHandlers::OPAQUE`
(no clone, `zend_objects_not_comparable`) cover the common tables. Use a
`static` for a table whose identity a handler checks (below).

`create_object` says php gives the class a `create_object` (its objects
carry native state). It is false only for a class that just replaces
`default_object_handlers` (`Directory`). `ReflectionClass::
newInstanceWithoutConstructor` refuses a final internal class that has
one. A class with a `create_object` and the default handlers registers
`ObjectHandlers::STD` (`Closure`, `HashContext`).

| php | rhp | called by |
|---|---|---|
| `get_properties_for` (and `get_debug_info`) | `get_properties_for(host, obj, purpose)`: `Some(table)` replaces the default view, `None` keeps it | `var_dump`/`print_r` (`Debug`, before `__debugInfo()`), `var_export`, `(array)`, `serialize` without `__serialize`/`__sleep`, `json_encode`, `get_object_vars` |
| `clone_obj` | `CloneObj::Std` (copy the properties; no payload), `Custom(f)` (`f(host, src, dst)` after the copy, before `__clone`), `None` (sets `ClassFlags::not_cloneable`) | `clone` |
| `compare` | `compare(host, op1, op2)`, operands in order; `Ok(None)` is `zend_std_compare_objects` | `==`, `<=>`, sorting, `in_array`… through `zend_compare`'s object branch: op1's handler when op1 is an object (a Closure op1 never asks op2), else op2's |
| `do_operation` | `do_operation(host, op, op1, op2)`: `Ok(None)` is FAILURE (the normal operator runs, casting with `cast_object`), an error is SUCCESS with an exception | `+ - * / **`: op1's handler when op1 is an object with one, else op2's, then both operands as numbers. `% << >> \| & ^` (`convert_op1_op2_long`) and `.` (`concat_function`): op1's handler, then op1 converted (to int / string; failing is "Unsupported operand types"), and only then op2's handler and op2. Compound assignments the same. `-$x` as `Mul` by -1, `~$x` as `BitNot` (else "Cannot perform bitwise not"); `++`/`--` as `Add`/`Sub` of 1, then `cast_object(Number)`, then php's "Cannot increment" TypeError |
| `cast_object` | `cast_object(host, obj, target)`: replaces php's default for every target, so hand the targets you do not convert to `host.std_cast_object(obj, target)` (`__toString()` for strings) | string conversion (`echo`, `.`, `(string)`), `(int)`/`(float)`/`intval`/`floatval` (`object_handlers::object_to_number`: a failure is php's "could not be converted" warning and 1), `(bool)`, `if`, `!`, `empty($o->p)`, `boolval`, `array_filter` (`object_handlers::is_true`), arithmetic operands and `array_sum`/`array_product` (`Number`), comparison with a scalar |
| `dtor_obj` | `dtor_obj(host, obj)`, once, before `__destruct` | the engine's destructor pass |
| `read_dimension`, `write_dimension`, `has_dimension`, `unset_dimension` | `dimension: Some(&DimensionHandlers { read, write, has, unset })`; `read` takes php's `BP_VAR_*` (`DimType`) and for `W`/`Rw` returns the stored slot as a `Value::Ref` | `$o[k]` reads, writes, `isset`/`empty`/`??`, `unset`, nested writes and `$o[k] =& $v` on that class, instead of its `ArrayAccess` methods (WeakMap) |
| `ce->get_iterator` | `get_iterator(host, obj, by_ref)`: an InternalIterator object over the class's `IteratorFuncs` (zend/internal_iterator.rs) | `foreach`, by value (through the iterator's methods) and by reference (`IteratorFuncs::current_ref`); `getIterator()` returns the same iterator (WeakMap, DatePeriod) |
| `write_property`, `unset_property` (the class's own part) | `write_property(host, obj, name)` / `unset_property(host, obj, name)`: a check run before the standard write or unset; an error stops it | every property write and unset on the class (PDOStatement's write-once `queryString`) |
| `free_obj` | the payload's `Drop`; work that may call PHP goes in `free_obj_host(host, obj)`, run once right after `__destruct` (in its place without one) unless the destructor kept a reference | the object's storage being freed; the engine's destructor pass |

Notes:

- A handler that starts with php's `ZEND_COMPARE_OBJECTS_FALLBACK` checks
  that both operands carry its table with
  `object_handlers::has_handlers(host, v, &MY_HANDLERS)` and returns
  `Ok(None)` otherwise. php compares the `compare` function pointers; rhp
  compares whole tables, so classes that php compares with each other
  must share one table (`DateTime` and `DateTimeImmutable` do; if they
  ever need separate tables, compare their `compare` fields instead).
- "op1" is the operand php's VM sees first. php swaps the operands of
  `*`, `|`, `&`, `^`, `==`, `!=`, `===` and `!==` when op1's kind
  (CONST < TMP < VAR < CV) is lower than op2's, so `[] | $g` reaches
  GMP's handler first and `$arr | $g` does not; rhp's compiler does the
  same (`php_swaps_operands`).
- Strictness: a handler that converts operands the way php's
  `ZEND_ARG_USES_STRICT_TYPES()` decides (GMP) asks
  `host.arg_uses_strict_types()`, not `caller_strict_types()`: php looks
  at the caller of the frame running the operator, which is weak when
  that caller is a builtin (a callback) or there is none (the main
  script).
- An exception raised while another is pending gets it as its previous
  one in php; a handler does that with `host.chain_thrown(new, pending)`.
- Formatters only see `&dyn Host`, so the dumpers resolve every reachable
  object's view up front (`var::resolve_views`).
- Weak references: freeing an object or closure that a WeakReference or
  WeakMap refers to (php's IS_OBJ_WEAKLY_REFERENCED, a flag on the object)
  calls zend/weakrefs.rs's `notify` from the object's `Drop`, after the
  payload and before the properties and the handle go (php's
  `zend_object_std_dtor`). Its registry of WeakReferences and WeakMaps by
  handle is php's `EG(weakrefs)`, the second exception to "no side
  tables"; it holds only `Weak` pointers and drops a handle's entry when
  that object is freed, so a reused handle never finds stale entries.
- Handlers not routed yet: property access (`read_property`,
  `has_property`…; writes and unsets only get the checks above), dimension access for classes other than WeakMap,
  `get_constructor` (`Directory`), `get_gc`. DOM's, SimpleXML's, XMLReader's and ZipArchive's properties (and
  SimpleXML's dimensions) go through their prelude classes'
  `__get`/`__set`/`__isset`/`__unset` and ArrayAccess methods, which call
  the ported handlers. DatePeriod declares its `@virtual` properties as
  property hooks that call them (get: the read handler, set: the read-only
  error), and its `unset_property` check refuses an unset; Reflection
  shows those properties as php's (virtual, no hooks) and traces leave the
  hook frames out. Two things make those stand-ins behave like php's
  handlers:
  - `host.magic_fetch()` says which fetch the running accessor serves:
    `Write` for a `__get`/`offsetGet` below a write (`$o->a->b = …`,
    `$o->a[] = …`, `$o[k]->p = …`, `&$o->a`, `unset($o->a->b)`; php's
    `BP_VAR_W`/`BP_VAR_UNSET`, where SimpleXML's `get_property_ptr_ptr`
    creates a missing element), `Empty` for the `__isset` or
    `offsetExists` of `empty()` (`check_empty`), else `Read`. Only the
    accessor's own frame sees the mode; whatever it calls sees `Read`.
  - Their frames never show in a trace: a built-in class's accessor whose
    php stub has no such method is engine plumbing, and a frame it calls
    gets the call site of the user frame below it. The same accessors, and
    a `__construct` standing in for php's `get_constructor` refusal
    (CurlHandle, XMLParser…), are left out of Reflection's method lists,
    `getConstructor()`, `get_class_methods()` and `method_exists()`
    (`Vm::hidden_method`). A stand-in
    `__call`/`__callStatic` (XMLReader's `open`/`XML`, which php's
    `get_method` runs statically or on an instance) shows as the stub
    method it runs, or not at all for a name the stub lacks.

  Known consequences until the engine routes real property and dimension
  handlers:
  - A user subclass that defines `__get`/`__set`/`__isset` (or
    `offsetGet`…) replaces the stand-in, so it shadows every virtual
    property, read-only checks included.
  - `property_exists()` and Reflection's property lists differ from php:
    they see no virtual properties of DOM, XMLReader and ZipArchive;
    SimpleXML implements `ArrayAccess`; `ReflectionMethod('XMLReader',
    'open')` and `method_exists()` find no `open`/`XML`
    (tests/api/known-differences.txt lists every such row).
  - `$sxe[null] = v` acts as `$sxe[] = v`: `offsetSet` cannot tell them
    apart.
  - `$sxe->new .= 'x'` and `$sxe->new++` on a missing element write the
    result through `__set`; php's `get_property_ptr_ptr` hands out a
    temporary and leaves an empty element.
- DateInterval keeps its fields as real properties instead, which its
  `get_properties_for` and methods read back into the payload
  (ext/date/objects.rs).
- php's `ce->unserialize` (the `C:` format) is a class field, not a
  handler: `unserialize` calls GMP's (`gmp::is_gmp_class`) or a
  `Serializable` class's `unserialize()` itself (ext/standard/var.rs).
  The SPL classes' old `C:` payloads are read by their prelude
  `unserialize()` methods through `__unserialize_value` /
  `__spl_array_unserialize`, which share the running unserialize()'s
  var_hash as php_var_unserialize does.
- `__PHP_Incomplete_Class` gets incomplete_class.c's handlers as checks
  on the VM's dynamic-property paths (`Vm::incomplete_class`): reads and
  `isset` warn, writes, unsets, write fetches and method calls throw.

## Class flags

`ClassFlags` carries php's `ce_flags` that change behaviour:

- `not_serializable` (`ZEND_ACC_NOT_SERIALIZABLE`) and `no_dynamic`
  (`ZEND_ACC_NO_DYNAMIC_PROPERTIES`) come from the stub's
  `@not-serializable` / `@strict-properties`, `allow_dynamic` also from
  `#[\AllowDynamicProperties]`: `gen signatures` writes them into the
  class table and the engine ORs them in when it links a built-in class.
  `not_serializable` and `allow_dynamic` are inherited, `no_dynamic` is
  not (zend_do_inheritance).
- `not_cloneable` is set from the handlers (`CloneObj::None`) and for
  enums, and is inherited with them.
- Every anonymous class is `not_serializable`, as zend_compile sets it.

`no_dynamic` applies wherever php creates a property: plain assignment,
and the fetches for a write or reference through a property (`$o->p[] =`,
`$o->p++`, `$o->p .=`, `&$o->p`). No engine code tests class names for
these.
