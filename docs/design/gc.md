# Cycle collection

Values are `Rc`s (value-model.md), so a reference cycle (`$o->self = $o`,
`$a[] = &$a`) keeps itself alive. rhp collects such garbage the way php
does (Zend/zend_gc.c): a synchronous trial-deletion collector over a
buffer of possible roots. `gc_collect_cycles()`, `gc_status()`,
`gc_enable()` / `gc_disable()` / `gc_enabled()` and `zend.enable_gc` are
php's.

## Possible roots (rhp-values `gc`)

php buffers an array or object when a reference to it goes away and its
count stays above zero (`gc_check_possible_root`). rhp calls
`gc::possible_root(&old)` at the same places, with the value about to be
dropped:

- a variable overwritten (`cv_assign`, `reg_set` of a CV, `=&`), unset, or
  freed with its frame (`Frame::drop`: the CVs, then the call's object:
  a closure call releases the Closure, a method call its `$this`; a
  method called from prelude code has no call in php and releases none);
- a lazy object's initializer when its info goes (php holds it twice, so
  it is a root just before it is freed);
- a property overwritten or unset, an array element replaced or removed
  (`PhpArray::insert` / `remove`, the cursor writes in lvalue.rs);
- the values an object, array or closure holds when it is freed
  (`ObjectData::drop`, `ArrayData::drop`, `ClosureData::drop`);
- `unserialize()`'s result (php roots it explicitly).

A temporary's release adds no root, as php's `zval_ptr_dtor_nogc`. A
reference adds the value it holds. The check is a tag test and
`Rc::strong_count > 1` inline, plus a `gc_buffered` flag on closures
(php's GC_INFO), so releasing a buffered closure stays out of the slow
path. Objects have no room for the flag (an object stays in the 96-byte
bin); the slow path remembers the object it last found buffered, so a
`$this` released call after call is cheap. The slow path
subtracts the reference the VM's destructor list holds (the `TRACKED`
flag). A free looks at the buffer only when some thread has something
buffered (one relaxed atomic load).

The buffer keeps php's layout: slots from 1, a LIFO list of free slots,
`first_unused`, php's two-finger compaction, the adaptive threshold
(10001, steps of 10000) and `buf_size` (16384, doubling). Entries are
`Weak`s. An object or closure is found by handle (a side table indexed by
object id), an array by address; a freed value leaves its slot from its
`Drop`, so `roots` and slot reuse follow php.

php collects inside the decrement that finds the buffer full. rhp cannot
(the drop may happen inside a builtin or a `Drop`), so the run is due at
the next instruction boundary: `MemState::tick` is the one flag the VM
loop reads per instruction (`attention || gc_due`). Roots added
meanwhile wait and enter the buffer after the run, and are held during it,
as php holds the value whose release triggered it.

Roots that waited for the due run enter the buffer after it without
checking the threshold again (php adds the triggering value directly), so
destructors that make new cycles cannot chain runs.

## The run (rhp-vm `gc.rs`)

The counts are `Rc::strong_count`s, read but never changed. A run:

1. walks the graph reachable from the roots, recording each node's edges:
   an object's properties, lazy initializer and instance, a generator's
   frame and values, a native class's `get_gc` handler; an array's
   elements; a reference's value; a closure's captures, bound `$this` and
   static variables;
2. gives each node `strong_count - edges into it from the graph - the
   references php does not have` (the destructor list's, and the hold on
   a destructed object, below);
3. colours black everything reachable from a node left above zero; the
   rest is white (garbage). This is php's mark grey / scan in one pass,
   with the same result;
4. continues as zend_gc_collect_cycles: black roots leave the buffer,
   white ones are collected in php's depth-first order (gc_add_garbage
   appends the nested garbage), the count is objects and arrays (not
   references).

A prelude class keeps its native storage in private `__*` array
properties; the walk goes through such an array to its elements (php's
get_gc lists them, and the C storage is not a counted zend_array) when
the property is its only holder and it is not buffered itself.

An edge the walk cannot see counts as an outside reference: the value is
kept, never freed early. Native payloads that hold values must list them
in `ObjectHandlers::get_gc` to be collectable.

When garbage has destructors (a `__destruct`, a native `dtor_obj`, or an
unfinished generator), php marks those objects, takes the garbage they
reach out of this run (`gc_remove_nested_data_from_buffer`, subtracted from
the count), calls the destructors in buffer order, frees the rest and runs
once more. rhp does the same. An object whose destructor left it with no
other reference has a count of zero in php but stays in the buffer, so the
rerun frees (and counts) it: rhp keeps a strong hold on such an object
until the rerun and subtracts it. Exceptions from the destructors are
chained (each later one gets the earlier as previous) and thrown from
`gc_collect_cycles()`.

Freeing goes in buffer order: an object's storage is released and its
handle returns to the free list right away (`rhp_values::gc_free_object`),
even while other garbage still holds the `Rc`; a garbage generator's state
goes with it; garbage references are emptied; arrays go when the last
holder does.

## Not done

- "Lazy object was released during initialization": php notices that the
  initializer dropped the last reference; rhp's callers hold their own
  `Rc`, so the count cannot tell (gh15999_001).
- foreach by reference iterates the variable live, without holding the
  reference php's iterator holds, so a loop over a cycle can be
  collected early (gc_047; foreach-semantics changes this).
- Live temporaries are not taken out of the buffer during a run and put
  back after (zend_gc_remove_root_tmpvars / check_root_tmpvars).

- Generators: php's generator destructor runs pending `finally` blocks;
  rhp's only drops the frame.
- Fibers: no destructor fiber. A suspended fiber's frames must be added
  to `Vm::gc_children` (as a generator's frame is) for a fiber in a cycle
  to be collectable.
- WeakMap entries: php's special edges (a WeakMap value kept alive only
  through its key) are not modelled; WeakMap / WeakReference hold strong
  references today.
- "GC buffer overflow (GC disabled)" at a billion roots.
