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

function iterator_to_array($iterator, $preserve_keys = true) {
    if (is_array($iterator)) { return $preserve_keys ? $iterator : array_values($iterator); }
    if ($iterator instanceof IteratorAggregate) { $iterator = $iterator->getIterator(); }
    $arr = [];
    foreach ($iterator as $k => $v) {
        if ($preserve_keys) { $arr[$k] = $v; } else { $arr[] = $v; }
    }
    return $arr;
}

function iterator_count($iterator) {
    if ($iterator instanceof IteratorAggregate) { $iterator = $iterator->getIterator(); }
    $count = 0;
    foreach ($iterator as $_) { $count++; }
    return $count;
}

function iterator_apply($iterator, $callback, $args = null) {
    if ($iterator instanceof IteratorAggregate) { $iterator = $iterator->getIterator(); }
    $count = 0;
    $iterator->rewind();
    while ($iterator->valid()) {
        if (call_user_func_array($callback, $args ?? []) === false) { break; }
        $count++;
        $iterator->next();
    }
    return $count;
}

class SplFileInfo implements Stringable {
    protected $__path;
    public function __construct($path) { $this->__path = $path; }
    public function getPathname() { return $this->__path; }
    public function getPath() { return dirname($this->__path); }
    public function getFilename() { return basename($this->__path); }
    public function getBasename($suffix = '') { return basename($this->__path, $suffix); }
    public function getExtension() { return pathinfo($this->__path, PATHINFO_EXTENSION); }
    public function getRealPath() { $r = realpath($this->__path); return $r === false ? false : $r; }
    public function isDir() { return is_dir($this->__path); }
    public function isFile() { return is_file($this->__path); }
    public function isLink() { return is_link($this->__path); }
    public function isReadable() { return is_readable($this->__path); }
    public function isWritable() { return is_writable($this->__path); }
    public function isExecutable() { return is_executable($this->__path); }
    public function getSize() { return filesize($this->__path); }
    public function getMTime() { return filemtime($this->__path); }
    public function getCTime() { return filectime($this->__path); }
    public function getATime() { return fileatime($this->__path); }
    public function getType() { return filetype($this->__path); }
    public function getPerms() { return fileperms($this->__path); }
    public function getPathInfo() { return new SplFileInfo(dirname($this->__path)); }
    public function getFileInfo() { return new SplFileInfo($this->__path); }
    public function __toString(): string { return (string) $this->__path; }
}

class FilesystemIterator extends SplFileInfo implements Iterator {
    const CURRENT_AS_PATHNAME = 32; const CURRENT_AS_FILEINFO = 0; const CURRENT_AS_SELF = 16;
    const KEY_AS_PATHNAME = 0; const KEY_AS_FILENAME = 256; const FOLLOW_SYMLINKS = 512;
    const NEW_CURRENT_AND_KEY = 256; const SKIP_DOTS = 4096; const UNIX_PATHS = 8192;
    protected $__dir; protected $__entries = []; protected $__pos = 0; protected $__flags;
    public function __construct($directory, $flags = 4096) {
        parent::__construct($directory);
        $this->__dir = rtrim($directory, '/');
        if ($this->__dir === '') { $this->__dir = $directory; }
        $this->__flags = $flags;
        $all = @scandir($directory);
        if ($all === false) { $all = []; }
        foreach ($all as $e) {
            if ($e === '.' || $e === '..') {
                if ($flags & self::SKIP_DOTS) { continue; }
            }
            $this->__entries[] = $e;
        }
    }
    public function rewind(): void { $this->__pos = 0; }
    public function valid(): bool { return $this->__pos < count($this->__entries); }
    public function next(): void { $this->__pos++; }
    public function getFilename() { return $this->__entries[$this->__pos] ?? ''; }
    public function getPathname() { return $this->__dir . '/' . $this->getFilename(); }
    public function getBasename($suffix = '') { return basename($this->getFilename(), $suffix); }
    public function getExtension() { return pathinfo($this->getFilename(), PATHINFO_EXTENSION); }
    public function isDir() { return is_dir($this->getPathname()); }
    public function isFile() { return is_file($this->getPathname()); }
    public function isLink() { return is_link($this->getPathname()); }
    public function getSize() { return filesize($this->getPathname()); }
    public function getRealPath() { $r = realpath($this->getPathname()); return $r === false ? false : $r; }
    public function key(): mixed {
        return ($this->__flags & self::KEY_AS_FILENAME) ? $this->getFilename() : $this->getPathname();
    }
    public function current(): mixed {
        if ($this->__flags & self::CURRENT_AS_PATHNAME) { return $this->getPathname(); }
        if ($this->__flags & self::CURRENT_AS_SELF) { return $this; }
        return new SplFileInfo($this->getPathname());
    }
}

class RecursiveDirectoryIterator extends FilesystemIterator implements RecursiveIterator {
    protected $__subPath = '';
    public function hasChildren(): bool {
        $n = $this->getFilename();
        if ($n === '.' || $n === '..') { return false; }
        return is_dir($this->getPathname());
    }
    public function getChildren(): RecursiveDirectoryIterator {
        $child = new RecursiveDirectoryIterator($this->getPathname(), $this->__flags);
        $child->__subPath = $this->getSubPathname();
        return $child;
    }
    public function getSubPath() { return $this->__subPath; }
    public function getSubPathname() {
        return ($this->__subPath === '' ? '' : $this->__subPath . '/') . $this->getFilename();
    }
    public function current(): mixed {
        if ($this->__flags & self::CURRENT_AS_PATHNAME) { return $this->getPathname(); }
        if ($this->__flags & self::CURRENT_AS_SELF) { return $this; }
        return new SplFileInfo($this->getPathname());
    }
}

class RecursiveIteratorIterator implements OuterIterator {
    const LEAVES_ONLY = 0; const SELF_FIRST = 1; const CHILD_FIRST = 2; const CATCH_GET_CHILD = 16;
    private $__root; private $__mode; private $__maxDepth = -1;
    private $__flat = []; private $__subs = []; private $__depths = []; private $__pos = 0;
    private $__curIt = null;
    public function __construct($it, $mode = 0, $flags = 0) {
        $this->__root = $it; $this->__mode = $mode;
    }
    public function setMaxDepth($maxDepth = -1): void { $this->__maxDepth = (int) $maxDepth; }
    public function getMaxDepth() { return $this->__maxDepth === -1 ? false : $this->__maxDepth; }
    public function rewind(): void {
        $this->__flat = []; $this->__subs = []; $this->__depths = [];
        $this->__build($this->__root, 0);
        $this->__pos = 0;
    }
    private function __build($it, $depth) {
        $it->rewind();
        while ($it->valid()) {
            $cur = $it->current();
            $sub = method_exists($it, 'getSubPathname') ? $it->getSubPathname() : '';
            $kids = $it->hasChildren() && ($this->__maxDepth === -1 || $depth < $this->__maxDepth);
            if ($this->__mode === self::CHILD_FIRST) {
                if ($kids) { $this->__build($it->getChildren(), $depth + 1); }
                $this->__push($cur, $sub, $depth, $it);
            } elseif ($this->__mode === self::SELF_FIRST) {
                $this->__push($cur, $sub, $depth, $it);
                if ($kids) { $this->__build($it->getChildren(), $depth + 1); }
            } else {
                if ($kids) { $this->__build($it->getChildren(), $depth + 1); }
                else { $this->__push($cur, $sub, $depth, $it); }
            }
            $it->next();
        }
    }
    private function __push($cur, $sub, $depth, $it) {
        $this->__flat[] = $cur; $this->__subs[] = $sub; $this->__depths[] = $depth;
    }
    public function valid(): bool { return $this->__pos < count($this->__flat); }
    public function current(): mixed { return $this->__flat[$this->__pos] ?? null; }
    public function key(): mixed {
        $c = $this->__flat[$this->__pos] ?? null;
        return is_object($c) && method_exists($c, 'getPathname') ? $c->getPathname() : $this->__pos;
    }
    public function next(): void { $this->__pos++; }
    public function getDepth() { return $this->__depths[$this->__pos] ?? 0; }
    public function getSubPathname() { return $this->__subs[$this->__pos] ?? ''; }
    public function getSubPath() {
        $s = $this->__subs[$this->__pos] ?? '';
        $p = strrpos($s, '/');
        return $p === false ? '' : substr($s, 0, $p);
    }
    public function getInnerIterator() { return $this->__root; }
}

class IteratorIterator implements OuterIterator {
    protected $__it;
    public function __construct($iterator, $class = null) {
        $this->__it = ($iterator instanceof IteratorAggregate) ? $iterator->getIterator() : $iterator;
    }
    public function getInnerIterator() { return $this->__it; }
    public function rewind(): void { $this->__it->rewind(); }
    public function valid(): bool { return $this->__it->valid(); }
    public function current(): mixed { return $this->__it->current(); }
    public function key(): mixed { return $this->__it->key(); }
    public function next(): void { $this->__it->next(); }
}

abstract class FilterIterator extends IteratorIterator {
    public function __construct(Iterator $iterator) { parent::__construct($iterator); }
    abstract public function accept(): bool;
    private function __fetch() {
        while ($this->__it->valid() && !$this->accept()) { $this->__it->next(); }
    }
    public function rewind(): void { $this->__it->rewind(); $this->__fetch(); }
    public function next(): void { $this->__it->next(); $this->__fetch(); }
}

class CallbackFilterIterator extends FilterIterator {
    protected $__cb;
    public function __construct(Iterator $iterator, callable $callback) {
        parent::__construct($iterator);
        $this->__cb = $callback;
    }
    public function accept(): bool {
        return (bool) call_user_func($this->__cb, $this->__it->current(), $this->__it->key(), $this->__it);
    }
}

abstract class RecursiveFilterIterator extends FilterIterator implements RecursiveIterator {
    public function __construct(RecursiveIterator $iterator) { parent::__construct($iterator); }
    public function hasChildren(): bool { return $this->__it->hasChildren(); }
    public function getChildren(): RecursiveFilterIterator {
        return new static($this->__it->getChildren());
    }
}

class RecursiveCallbackFilterIterator extends RecursiveFilterIterator {
    protected $__cb;
    public function __construct(RecursiveIterator $iterator, callable $callback) {
        parent::__construct($iterator);
        $this->__cb = $callback;
    }
    public function accept(): bool {
        return (bool) call_user_func($this->__cb, $this->__it->current(), $this->__it->key(), $this->__it);
    }
    public function getChildren(): RecursiveCallbackFilterIterator {
        return new RecursiveCallbackFilterIterator($this->__it->getChildren(), $this->__cb);
    }
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
