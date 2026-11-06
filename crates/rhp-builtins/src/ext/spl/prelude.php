
interface SeekableIterator extends Iterator { public function seek(int $offset): void; }
class ArrayIterator implements SeekableIterator, ArrayAccess, Countable {
    const STD_PROP_LIST = 1;
    const ARRAY_AS_PROPS = 2;
    private $__s; private $__f = 0; private $__k; private $__p = 0;
    public function __construct($array = [], int $flags = 0) { $this->__s = is_object($array) ? get_object_vars($array) : $array; $this->__f = $flags; $this->__k = array_keys($this->__s); }
    public function current(): mixed { $k = $this->__k[$this->__p] ?? null; return $k === null ? null : ($this->__s[$k] ?? null); }
    public function key(): mixed { return $this->__k[$this->__p] ?? null; }
    public function next(): void { $this->__p++; }
    public function rewind(): void { $this->__k = array_keys($this->__s); $this->__p = 0; }
    public function valid(): bool { return $this->__p < count($this->__k); }
    public function seek(int $offset): void {
        if ($offset < 0 || $offset >= count($this->__s)) { throw new OutOfBoundsException("Seek position $offset is out of range"); }
        $this->__k = array_keys($this->__s); $this->__p = $offset;
    }
    public function count(): int { return count($this->__s); }
    public function offsetExists($k): bool { return isset($this->__s[$k]); }
    public function offsetGet($k): mixed { return $this->__s[$k]; }
    public function offsetSet($k, $v): void {
        if ($k === null) { $this->__s[] = $v; $this->__k[] = array_key_last($this->__s); }
        else { $new = !array_key_exists($k, $this->__s); $this->__s[$k] = $v; if ($new) { $this->__k[] = array_key_last($this->__s); } }
    }
    public function offsetUnset($k): void {
        if (!array_key_exists($k, $this->__s)) { return; }
        unset($this->__s[$k]);
        $i = array_search($k, $this->__k, true);
        if ($i !== false) { array_splice($this->__k, $i, 1); if ($i < $this->__p) { $this->__p--; } }
    }
    public function getArrayCopy(): array { return $this->__s; }
    public function append($v): void { $this->offsetSet(null, $v); }
    public function getFlags(): int { return $this->__f; }
    public function setFlags(int $flags): void { $this->__f = $flags; }
    public function asort(int $flags = SORT_REGULAR): true { asort($this->__s, $flags); return true; }
    public function ksort(int $flags = SORT_REGULAR): true { ksort($this->__s, $flags); return true; }
    public function uasort(callable $callback): true { uasort($this->__s, $callback); return true; }
    public function uksort(callable $callback): true { uksort($this->__s, $callback); return true; }
    public function natsort(): true { natsort($this->__s); return true; }
    public function natcasesort(): true { natcasesort($this->__s); return true; }
    public function __serialize(): array { return [$this->__f & 0xffff, $this->__s, get_mangled_object_vars($this), null]; }
    public function __unserialize(array $data): void {
        $this->__f = $data[0] ?? 0;
        $this->__s = $data[1] ?? [];
        foreach ($data[2] ?? [] as $k => $v) {
            $k = (string)$k;
            if ($k !== '' && $k[0] === "\0") {
                $p = strpos($k, "\0", 1); $c = substr($k, 1, $p - 1); $n = substr($k, $p + 1);
                __reflect_set_prop($this, $n, $v, $c === '*' ? static::class : $c);
            } else { $this->$k = $v; }
        }
    }
}
class ArrayObject implements IteratorAggregate, ArrayAccess, Countable {
    const STD_PROP_LIST = 1;
    const ARRAY_AS_PROPS = 2;
    private $__s; private $__f = 0; private $__i = 'ArrayIterator';
    public function __construct($array = [], int $flags = 0, string $iteratorClass = 'ArrayIterator') { $this->__s = is_object($array) ? get_object_vars($array) : $array; $this->__f = $flags; $this->__i = $iteratorClass; }
    public function getIterator(): Iterator { $c = $this->__i; return new $c($this->__s); }
    public function exchangeArray($array): array { $old = $this->__s; $this->__s = is_object($array) ? get_object_vars($array) : $array; return $old; }
    public function getIteratorClass(): string { return $this->__i; }
    public function setIteratorClass(string $iteratorClass): void { $this->__i = $iteratorClass; }
    public function count(): int { return count($this->__s); }
    public function offsetExists($k): bool { return isset($this->__s[$k]); }
    public function offsetGet($k): mixed { return $this->__s[$k]; }
    public function offsetSet($k, $v): void { if ($k === null) { $this->__s[] = $v; } else { $this->__s[$k] = $v; } }
    public function offsetUnset($k): void { unset($this->__s[$k]); }
    public function getArrayCopy(): array { return $this->__s; }
    public function append($v): void { $this->__s[] = $v; }
    public function getFlags(): int { return $this->__f; }
    public function setFlags(int $flags): void { $this->__f = $flags; }
    public function asort(int $flags = SORT_REGULAR): true { asort($this->__s, $flags); return true; }
    public function ksort(int $flags = SORT_REGULAR): true { ksort($this->__s, $flags); return true; }
    public function uasort(callable $callback): true { uasort($this->__s, $callback); return true; }
    public function uksort(callable $callback): true { uksort($this->__s, $callback); return true; }
    public function natsort(): true { natsort($this->__s); return true; }
    public function natcasesort(): true { natcasesort($this->__s); return true; }
    public function __serialize(): array { return [$this->__f & 0xffff, $this->__s, get_mangled_object_vars($this), $this->__i === 'ArrayIterator' ? null : $this->__i]; }
    public function __unserialize(array $data): void {
        $this->__f = $data[0] ?? 0;
        $this->__s = $data[1] ?? [];
        foreach ($data[2] ?? [] as $k => $v) {
            $k = (string)$k;
            if ($k !== '' && $k[0] === "\0") {
                $p = strpos($k, "\0", 1); $c = substr($k, 1, $p - 1); $n = substr($k, $p + 1);
                __reflect_set_prop($this, $n, $v, $c === '*' ? static::class : $c);
            } else { $this->$k = $v; }
        }
        if (isset($data[3])) { $this->__i = $data[3]; }
    }
}

class SplDoublyLinkedList implements Iterator, Countable, ArrayAccess {
    const IT_MODE_LIFO = 2; const IT_MODE_FIFO = 0; const IT_MODE_DELETE = 1; const IT_MODE_KEEP = 0;
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
    // The removed element is kept until the splice is done (a destructor may
    // use the list).
    public function offsetUnset($i): void { $gone = array_splice($this->__d, $i, 1); }
    public function setIteratorMode($mode): void { $this->__lifo = (bool)($mode & 2); $this->__del = (bool)($mode & 1); }
    protected $__del = false;
    public function __debugInfo(): array {
        $f = ($this->__lifo ? 2 : 0) | ($this->__del ? 1 : 0) | ($this instanceof SplQueue || $this instanceof SplStack ? 4 : 0);
        return __spl_user_props($this) + ["\0SplDoublyLinkedList\0flags" => $f, "\0SplDoublyLinkedList\0dllist" => $this->__d];
    }
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

final class WeakReference {
    public $__o = null;
    private static $__make = false;
    public function __construct() {
        if (!self::$__make) { throw new Error('Direct instantiation of WeakReference is not allowed, use WeakReference::create instead'); }
    }
    public static function create($object) { self::$__make = true; $r = new WeakReference(); self::$__make = false; $r->__o = $object; return $r; }
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

class SplFixedArray implements IteratorAggregate, ArrayAccess, Countable, JsonSerializable {
    private $__d = [];
    private $__size = 0;
    public function __construct(int $size = 0) {
        if ($size < 0) { throw new ValueError('SplFixedArray::__construct(): Argument #1 ($size) must be greater than or equal to 0'); }
        $this->__size = $size; $this->__d = $size > 0 ? array_fill(0, $size, null) : [];
    }
    /* spl_offset_convert_to_ulong: an index as php converts it, or a TypeError. */
    private static function __idx($i): int {
        if (is_int($i)) { return $i; }
        if (is_bool($i)) { return (int) $i; }
        if (is_float($i)) {
            if (!is_finite($i) || $i >= 9.2233720368547758E+18 || $i < -9.2233720368547758E+18) {
                return (int) $i; // the cast raises php's out-of-range warning
            }
            if ($i != (int) $i) {
                __stdext_deprecated('Implicit conversion from float ' . var_export($i, true) . ' to int loses precision');
            }
            return (int) $i;
        }
        if (is_string($i) && preg_match('/^(0|-?[1-9][0-9]*)$/D', $i) && (string) (int) $i === $i) { return (int) $i; }
        if (is_resource($i)) { return (int) $i; }
        throw new TypeError('Cannot access offset of type ' . get_debug_type($i) . ' on SplFixedArray');
    }
    public function count(): int { return $this->__size; }
    public function getSize(): int { return $this->__size; }
    public function setSize(int $size): bool {
        if ($size < 0) { throw new ValueError('SplFixedArray::setSize(): Argument #1 ($size) must be greater than or equal to 0'); }
        if ($size < $this->__size) {
            // spl_fixedarray_resize: the array is cut to its new size before
            // the dropped elements are destroyed, so their destructors see
            // (and may resize) the shrunk array.
            $dropped = array_slice($this->__d, $size);
            $this->__d = array_slice($this->__d, 0, $size);
            $this->__size = $size;
            unset($dropped);
        } elseif ($size > $this->__size) {
            $this->__d = array_merge($this->__d, array_fill(0, $size - $this->__size, null));
            $this->__size = $size;
        }
        return true;
    }
    public function toArray(): array { return $this->__d; }
    public function jsonSerialize(): array { return $this->__d; }
    public static function fromArray(array $array, bool $preserveKeys = true): SplFixedArray {
        $a = new SplFixedArray();
        if ($array === []) { return $a; }
        if ($preserveKeys) {
            $max = -1;
            foreach ($array as $k => $_) {
                if (!is_int($k) || $k < 0) { throw new InvalidArgumentException('array must contain only positive integer keys'); }
                if ($k > $max) { $max = $k; }
            }
            $a->__d = array_fill(0, $max + 1, null);
            foreach ($array as $k => $v) { $a->__d[$k] = $v; }
            $a->__size = $max + 1;
        } else {
            $a->__d = array_values($array);
            $a->__size = count($array);
        }
        return $a;
    }
    public function offsetExists($index): bool {
        $i = self::__idx($index);
        return $i >= 0 && $i < $this->__size && $this->__d[$i] !== null;
    }
    public function offsetGet($index): mixed {
        $i = self::__idx($index);
        if ($i < 0 || $i >= $this->__size) { throw new OutOfBoundsException('Index invalid or out of range'); }
        return $this->__d[$i];
    }
    public function offsetSet($index, mixed $value): void {
        if ($index === null) { throw new Error('[] operator not supported for SplFixedArray'); }
        $i = self::__idx($index);
        if ($i < 0 || $i >= $this->__size) { throw new OutOfBoundsException('Index invalid or out of range'); }
        $this->__d[$i] = $value;
    }
    public function offsetUnset($index): void {
        $i = self::__idx($index);
        if ($i < 0 || $i >= $this->__size) { throw new OutOfBoundsException('Index invalid or out of range'); }
        $this->__d[$i] = null;
    }
    // php's InternalIterator reads the live elements: writes made while
    // iterating (php-cs-fixer's Tokens) are seen.
    public function getIterator(): Iterator {
        for ($i = 0; $i < $this->__size; $i++) { yield $i => $this->__d[$i]; }
    }
    public function __debugInfo(): array {
        $r = $this->__d;
        foreach (__spl_user_props($this) as $k => $v) { $r[$k] = $v; }
        return $r;
    }
    public function __serialize(): array { return $this->__d; }
    public function __unserialize(array $data): void {
        // ZVAL_COPY_DEREF: the elements' values, not references.
        $d = [];
        foreach ($data as $v) { $d[] = $v; }
        $this->__d = $d; $this->__size = count($data);
    }
}