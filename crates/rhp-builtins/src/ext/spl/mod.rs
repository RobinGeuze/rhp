//! A slice of the `SPL` extension: the autoload registry
//! (`spl_autoload_register` and friends). The callables live on the [`Host`]
//! (the VM), which invokes them when a class reference fails to resolve.

use std::collections::HashMap;

use rhp_values::{conv, PhpArray, Value};

use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

/// `spl_autoload_register(?callable $callback = null, bool $throw = true, bool $prepend = false): bool`
fn fn_spl_autoload_register(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match args.first().map(deref) {
        // The default SPL autoloader is not supported; registering it is a no-op.
        None | Some(Value::Null) => Ok(Value::Bool(true)),
        Some(cb) => {
            if !host.is_callable_value(&cb) {
                return Err(Thrown::error(
                    "TypeError",
                    "spl_autoload_register(): Argument #1 ($callback) must be a valid callback",
                ));
            }
            let prepend = args.get(2).map(|v| conv::to_bool(&deref(v))).unwrap_or(false);
            host.register_autoloader(cb, prepend);
            Ok(Value::Bool(true))
        }
    }
}

/// `spl_autoload_unregister(callable $callback): bool` — not yet supported
/// (callable identity comparison); reports success without removing.
fn fn_spl_autoload_unregister(_host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}

/// `spl_autoload_functions(): array`
fn fn_spl_autoload_functions(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    for cb in host.autoload_functions() {
        a.push(cb);
    }
    Ok(Value::Array(a))
}

/// `spl_autoload_call(string $class): void`
fn fn_spl_autoload_call(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = conv::to_string_bytes(&deref(&args[0]));
    host.autoload_call(&name)?;
    Ok(Value::Null)
}

/// `spl_object_id(object $obj): int` — the object's per-run handle id.
fn fn_spl_object_id(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match deref(&args[0]) {
        Value::Object(h) => Ok(Value::Int(h.borrow().id as i64)),
        _ => Err(Thrown::type_error(
            "spl_object_id(): Argument #1 ($object) must be of type object",
        )),
    }
}

/// `spl_object_hash(object $obj): string` — a 32-hex-char id unique to the
/// object for its lifetime. (Derived from the handle id; not byte-identical to
/// php's hash, but stable and collision-free as a map key.)
fn fn_spl_object_hash(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match deref(&args[0]) {
        Value::Object(h) => {
            let id = h.borrow().id;
            Ok(Value::str(format!("{id:032x}").into_bytes()))
        }
        _ => Err(Thrown::type_error(
            "spl_object_hash(): Argument #1 ($object) must be of type object",
        )),
    }
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(name, Builtin { name, func, min_args: min, max_args: max });
    }
    put(m, "spl_autoload_register", fn_spl_autoload_register, 0, Some(3));
    put(m, "spl_autoload_unregister", fn_spl_autoload_unregister, 1, Some(1));
    put(m, "spl_autoload_functions", fn_spl_autoload_functions, 0, Some(0));
    put(m, "spl_autoload_call", fn_spl_autoload_call, 1, Some(1));
    put(m, "spl_object_id", fn_spl_object_id, 1, Some(1));
    put(m, "spl_object_hash", fn_spl_object_hash, 1, Some(1));
}

/// SPL / core data-structure classes implemented in PHP (ArrayIterator and
/// friends), backed by a plain array.
pub const PRELUDE_PHP: &str = r#"
class ArrayIterator implements Iterator, ArrayAccess, Countable {
    private $__s; private $__k; private $__p = 0;
    public function __construct($array = [], $flags = 0) { $this->__s = $array; $this->__k = array_keys($array); }
    public function current(): mixed { return $this->__s[$this->__k[$this->__p]] ?? null; }
    public function key(): mixed { return $this->__k[$this->__p] ?? null; }
    public function next(): void { $this->__p++; }
    public function rewind(): void { $this->__k = array_keys($this->__s); $this->__p = 0; }
    public function valid(): bool { return $this->__p < count($this->__k); }
    public function count(): int { return count($this->__s); }
    public function offsetExists($k): bool { return isset($this->__s[$k]); }
    public function offsetGet($k): mixed { return $this->__s[$k] ?? null; }
    public function offsetSet($k, $v): void { if ($k === null) { $this->__s[] = $v; } else { $this->__s[$k] = $v; } }
    public function offsetUnset($k): void { unset($this->__s[$k]); }
    public function getArrayCopy() { return $this->__s; }
    public function append($v) { $this->__s[] = $v; }
}
class ArrayObject implements IteratorAggregate, ArrayAccess, Countable {
    private $__s;
    public function __construct($array = [], $flags = 0) { $this->__s = $array; }
    public function getIterator(): Iterator { return new ArrayIterator($this->__s); }
    public function count(): int { return count($this->__s); }
    public function offsetExists($k): bool { return isset($this->__s[$k]); }
    public function offsetGet($k): mixed { return $this->__s[$k] ?? null; }
    public function offsetSet($k, $v): void { if ($k === null) { $this->__s[] = $v; } else { $this->__s[$k] = $v; } }
    public function offsetUnset($k): void { unset($this->__s[$k]); }
    public function getArrayCopy() { return $this->__s; }
    public function append($v) { $this->__s[] = $v; }
}

class SplDoublyLinkedList implements Iterator, Countable, ArrayAccess {
    protected $__d = [];
    protected $__p = 0;
    protected $__lifo = false;
    public function push($v): void { $this->__d[] = $v; }
    public function pop(): mixed { if (count($this->__d) === 0) { throw new RuntimeException("Can't pop from an empty datastructure"); } return array_pop($this->__d); }
    public function shift(): mixed { if (count($this->__d) === 0) { throw new RuntimeException("Can't shift from an empty datastructure"); } return array_shift($this->__d); }
    public function unshift($v): void { array_unshift($this->__d, $v); }
    public function top(): mixed { if (count($this->__d) === 0) { throw new RuntimeException("Peek at an empty datastructure"); } return $this->__d[count($this->__d) - 1]; }
    public function bottom(): mixed { if (count($this->__d) === 0) { throw new RuntimeException("Peek at an empty datastructure"); } return $this->__d[0]; }
    public function add($index, $value): void { array_splice($this->__d, $index, 0, [$value]); }
    public function isEmpty(): bool { return count($this->__d) === 0; }
    public function count(): int { return count($this->__d); }
    public function toArray(): array { return $this->__d; }
    public function offsetExists($i): bool { return isset($this->__d[$i]); }
    public function offsetGet($i): mixed { return $this->__d[$i]; }
    public function offsetSet($i, $v): void { if ($i === null) { $this->__d[] = $v; } else { $this->__d[$i] = $v; } }
    public function offsetUnset($i): void { array_splice($this->__d, $i, 1); }
    public function setIteratorMode($mode): void { $this->__lifo = (bool)($mode & 2); }
    public function rewind(): void { $this->__p = $this->__lifo ? count($this->__d) - 1 : 0; }
    public function current(): mixed { return $this->__d[$this->__p] ?? null; }
    public function key(): mixed { return $this->__p; }
    public function next(): void { if ($this->__lifo) { $this->__p--; } else { $this->__p++; } }
    public function valid(): bool { return $this->__p >= 0 && $this->__p < count($this->__d); }
}

class SplQueue extends SplDoublyLinkedList {
    public function enqueue($v): void { $this->push($v); }
    public function dequeue(): mixed { return $this->shift(); }
}

class SplStack extends SplDoublyLinkedList {
    public function __construct() { $this->__lifo = true; }
}

class SplObjectStorage implements Countable, Iterator, ArrayAccess {
    private $__o = [];
    private $__v = [];
    private $__k = [];
    private $__p = 0;
    public function attach($obj, $data = null): void { $id = spl_object_id($obj); $this->__o[$id] = $obj; $this->__v[$id] = $data; }
    public function detach($obj): void { $id = spl_object_id($obj); unset($this->__o[$id], $this->__v[$id]); }
    public function contains($obj): bool { return isset($this->__o[spl_object_id($obj)]); }
    public function count(): int { return count($this->__o); }
    public function offsetExists($obj): bool { return $this->contains($obj); }
    public function offsetGet($obj): mixed { return $this->__v[spl_object_id($obj)] ?? null; }
    public function offsetSet($obj, $data): void { $this->attach($obj, $data); }
    public function offsetUnset($obj): void { $this->detach($obj); }
    public function getInfo(): mixed { return $this->__v[$this->__k[$this->__p]] ?? null; }
    public function setInfo($data): void { if (isset($this->__k[$this->__p])) { $this->__v[$this->__k[$this->__p]] = $data; } }
    public function rewind(): void { $this->__k = array_keys($this->__o); $this->__p = 0; }
    public function current(): mixed { return $this->__o[$this->__k[$this->__p]] ?? null; }
    public function key(): mixed { return $this->__p; }
    public function next(): void { $this->__p++; }
    public function valid(): bool { return $this->__p < count($this->__k); }
}

class SplPriorityQueue implements Countable, Iterator {
    private $__items = [];
    private $__serial = 0;
    public function insert($value, $priority): bool {
        $this->__items[] = ['data' => $value, 'priority' => $priority, 'serial' => $this->__serial--];
        usort($this->__items, function ($a, $b) {
            $c = $a['priority'] <=> $b['priority'];
            return $c !== 0 ? $c : ($a['serial'] <=> $b['serial']);
        });
        return true;
    }
    public function extract(): mixed { $it = array_pop($this->__items); return $it === null ? null : $it['data']; }
    public function top(): mixed { if (count($this->__items) === 0) { throw new RuntimeException("Can't peek at an empty heap"); } return $this->__items[count($this->__items) - 1]['data']; }
    public function peek(): mixed { return $this->top(); }
    public function count(): int { return count($this->__items); }
    public function isEmpty(): bool { return count($this->__items) === 0; }
    public function rewind(): void {}
    public function current(): mixed { return $this->top(); }
    public function key(): mixed { return count($this->__items) - 1; }
    public function next(): void { $this->extract(); }
    public function valid(): bool { return count($this->__items) > 0; }
}

final class WeakReference {
    public $__o = null;
    private function __construct() {}
    public static function create($object) { $r = new WeakReference(); $r->__o = $object; return $r; }
    public function get() { return $this->__o; }
}

final class WeakMap implements ArrayAccess, Countable, IteratorAggregate {
    private $__o = [];
    private $__v = [];
    public function offsetExists($obj): bool { return isset($this->__o[spl_object_id($obj)]); }
    public function offsetGet($obj): mixed { return $this->__v[spl_object_id($obj)]; }
    public function offsetSet($obj, $value): void { $id = spl_object_id($obj); $this->__o[$id] = $obj; $this->__v[$id] = $value; }
    public function offsetUnset($obj): void { $id = spl_object_id($obj); unset($this->__o[$id], $this->__v[$id]); }
    public function count(): int { return count($this->__o); }
    public function getIterator(): Iterator {
        $pairs = [];
        foreach ($this->__o as $id => $obj) { $pairs[] = [$obj, $this->__v[$id]]; }
        return new ArrayIterator($pairs);
    }
}

class SplFixedArray implements ArrayAccess, Countable, Iterator {
    private $__d = [];
    private $__size = 0;
    private $__p = 0;
    public function __construct($size = 0) { $this->__size = $size; for ($i = 0; $i < $size; $i++) { $this->__d[$i] = null; } }
    public function count(): int { return $this->__size; }
    public function getSize(): int { return $this->__size; }
    public function setSize($size): void {
        if ($size < $this->__size) { for ($i = $size; $i < $this->__size; $i++) { unset($this->__d[$i]); } }
        else { for ($i = $this->__size; $i < $size; $i++) { $this->__d[$i] = null; } }
        $this->__size = $size;
    }
    public function toArray(): array { return $this->__d; }
    public static function fromArray($array) { $a = new SplFixedArray(count($array)); $i = 0; foreach ($array as $v) { $a[$i++] = $v; } return $a; }
    public function offsetExists($i): bool { return $i >= 0 && $i < $this->__size; }
    public function offsetGet($i): mixed { if ($i < 0 || $i >= $this->__size) { throw new RuntimeException("Index invalid or out of range"); } return $this->__d[$i]; }
    public function offsetSet($i, $v): void { if ($i < 0 || $i >= $this->__size) { throw new RuntimeException("Index invalid or out of range"); } $this->__d[$i] = $v; }
    public function offsetUnset($i): void { if ($i >= 0 && $i < $this->__size) { $this->__d[$i] = null; } }
    public function rewind(): void { $this->__p = 0; }
    public function current(): mixed { return $this->__d[$this->__p] ?? null; }
    public function key(): mixed { return $this->__p; }
    public function next(): void { $this->__p++; }
    public function valid(): bool { return $this->__p < $this->__size; }
}
"#;
