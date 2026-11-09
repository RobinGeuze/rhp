
interface SeekableIterator extends Iterator { public function seek(int $offset): void; }
class ArrayIterator implements SeekableIterator, ArrayAccess, Serializable, Countable {
    const STD_PROP_LIST = 1;
    const ARRAY_AS_PROPS = 2;
    private $__s = []; private $__f = 0; private $__k = []; private $__p = 0;
    // The backing object (foreach by reference reaches its property slots).
    private $__o = null;
    // The ArrayObject / ArrayIterator whose storage this one shares.
    private $__w = null;
    public function __construct($array = [], int $flags = 0) { $this->__o = is_object($array) && !($array instanceof ArrayObject || $array instanceof ArrayIterator) ? $array : null; if (is_object($array)) { __spl_deprecated('ArrayIterator::__construct(): Using an object as a backing array for ' . ($this instanceof ArrayIterator ? 'ArrayIterator' : 'ArrayObject') . ' is deprecated, as it allows violating class constraints and invariants'); } self::__store($this, $array); $this->__f = $flags; $this->__k = array_keys($this->__s); }
    // The storage of an ArrayObject / ArrayIterator over $array: another
    // one's storage (shared, shown as that object), else the array or the
    // object's properties.
    private static function __store($me, $array): void {
        __spl_array_bind($me, null);
        $me->__w = null;
        if (($array instanceof ArrayObject || $array instanceof ArrayIterator) && $array !== $me) { __spl_array_bind($me, $array); $me->__w = $array; }
        else { $me->__s = is_object($array) ? get_object_vars($array) : $array; }
    }
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
    // spl_array_has_dimension_ex(check_empty=2): the key exists, even
    // with a null value.
    public function offsetExists($k): bool { return isset($this->__s[$k]) || ($k === null || is_scalar($k)) && array_key_exists($k === null ? '' : (is_float($k) || is_bool($k) ? (int) $k : $k), $this->__s); }
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
    // spl_array_method parses the callback as "z": uasort() checks it.
    public function uasort($callback): true { uasort($this->__s, $callback); return true; }
    public function uksort($callback): true { uksort($this->__s, $callback); return true; }
    public function natsort(): true { natsort($this->__s); return true; }
    public function natcasesort(): true { natcasesort($this->__s); return true; }
    public function __serialize(): array { return [$this->__f & 0xffff, $this->__w ?? $this->__s, get_mangled_object_vars($this), null]; }
    public function __unserialize(array $data): void {
        self::__checkData($data);
        $this->__f = $data[0];
        if (is_object($data[1])) { __spl_deprecated('ArrayIterator::__unserialize(): Using an object as a backing array for ' . ($this instanceof ArrayIterator ? 'ArrayIterator' : 'ArrayObject') . ' is deprecated, as it allows violating class constraints and invariants'); }
        self::__store($this, $data[1]);
        $this->__loadMembers($data[2]);
    }
    // ArrayObject::__unserialize's checks.
    private static function __checkData(array $data): void {
        if (!isset($data[0]) || !array_key_exists(1, $data) || !isset($data[2]) || !is_int($data[0]) || !is_array($data[2])
                || (isset($data[3]) && !is_string($data[3]))) {
            throw new UnexpectedValueException('Incomplete or ill-typed serialization data');
        }
        if (!is_array($data[1]) && !is_object($data[1])) {
            throw new InvalidArgumentException('Passed variable is not an array or object');
        }
    }
    // object_properties_load: members keyed by mangled names.
    private function __loadMembers(array $m): void { __object_properties_load($this, $m); }
    public function serialize(): string {
        __serialize_init();
        try {
            $s = 'x:' . serialize($this->__f & 0x0100ffff);
            if ($this->__o !== $this) { $s .= serialize($this->__o ?? $this->__w ?? $this->__s) . ';'; }
            $s .= 'm:' . serialize(get_mangled_object_vars($this));
        } finally { __serialize_destroy(); }
        return $s;
    }
    public function __debugInfo(): array { return __spl_array_debug_info($this); }
    public function unserialize(string $data): void {
        [$f, $s, $m] = __spl_array_unserialize($data, 'ArrayIterator::unserialize');
        if ($f === null) { return; }
        if (is_object($s)) { __spl_deprecated('ArrayIterator::unserialize(): Using an object as a backing array for ' . ($this instanceof ArrayIterator ? 'ArrayIterator' : 'ArrayObject') . ' is deprecated, as it allows violating class constraints and invariants'); $s = get_object_vars($s); }
        $this->__f = $f; if ($s !== null) { $this->__s = $s; $this->__k = array_keys($s); }
        $this->__loadMembers($m);
    }
}
class ArrayObject implements IteratorAggregate, ArrayAccess, Serializable, Countable {
    const STD_PROP_LIST = 1;
    const ARRAY_AS_PROPS = 2;
    private $__s = []; private $__f = 0; private $__i = 'ArrayIterator';
    // The backing object, whose property table offset writes also update
    // (not another ArrayObject / ArrayIterator: php uses its storage).
    private $__o = null;
    // The ArrayObject / ArrayIterator whose storage this one shares.
    private $__w = null;
    private static function __store($me, $array): void {
        __spl_array_bind($me, null);
        $me->__w = null;
        if (($array instanceof ArrayObject || $array instanceof ArrayIterator) && $array !== $me) { __spl_array_bind($me, $array); $me->__w = $array; }
        else { $me->__s = is_object($array) ? get_object_vars($array) : $array; }
    }
    private static function __backing($a, $self) { return is_object($a) && ($a === $self || !($a instanceof ArrayObject || $a instanceof ArrayIterator)) ? $a : null; }
    public function __construct($array = [], int $flags = 0, string $iteratorClass = 'ArrayIterator') { $this->__o = self::__backing($array, $this); if (is_object($array)) { __spl_deprecated('ArrayObject::__construct(): Using an object as a backing array for ' . ($this instanceof ArrayIterator ? 'ArrayIterator' : 'ArrayObject') . ' is deprecated, as it allows violating class constraints and invariants'); } self::__store($this, $array); $this->__f = $flags; $this->__i = $iteratorClass; }
    public function getIterator(): Iterator { $c = $this->__i; return new $c($this->__s); }
    public function exchangeArray($array): array { if (is_object($array)) { __spl_deprecated('ArrayObject::exchangeArray(): Using an object as a backing array for ' . ($this instanceof ArrayIterator ? 'ArrayIterator' : 'ArrayObject') . ' is deprecated, as it allows violating class constraints and invariants'); } $old = $this->__s; $this->__o = self::__backing($array, $this); self::__store($this, $array); return $old; }
    public function getIteratorClass(): string { return $this->__i; }
    public function setIteratorClass(string $iteratorClass): void { $this->__i = $iteratorClass; }
    public function count(): int { return count($this->__s); }
    // spl_array_has_dimension_ex(check_empty=2): the key exists, even
    // with a null value.
    public function offsetExists($k): bool { return isset($this->__s[$k]) || ($k === null || is_scalar($k)) && array_key_exists($k === null ? '' : (is_float($k) || is_bool($k) ? (int) $k : $k), $this->__s); }
    public function offsetGet($k): mixed { if (is_array($k) || is_object($k)) { self::__badKey($k, $this); } return $this->__s[$k]; }
    public function offsetSet($k, $v): void { if ($k === null) { $this->__s[] = $v; if ($this->__o !== null) { __spl_obj_store($this->__o, array_key_last($this->__s), $v); } } else { if (is_array($k) || is_object($k)) { self::__badKey($k, $this); } $this->__s[$k] = $v; if ($this->__o !== null) { __spl_obj_store($this->__o, $k, $v); } } }
    // spl_array get_hash_key's failure (zend_illegal_container_offset).
    private static function __badKey($k, $o): never { throw new TypeError('Cannot access offset of type ' . (is_object($k) ? get_class($k) : 'array') . ' on ' . get_class($o)); }
    public function offsetUnset($k): void { unset($this->__s[$k]); if ($this->__o !== null) { __spl_obj_store($this->__o, $k, null, true); } }
    public function getArrayCopy(): array { return $this->__s; }
    public function append($v): void { if ($this->__o !== null) { throw new Error('Cannot append properties to objects, use ' . get_class($this) . '::offsetSet() instead'); } $this->__s[] = $v; }
    public function getFlags(): int { return $this->__f; }
    public function setFlags(int $flags): void { $this->__f = $flags; }
    public function asort(int $flags = SORT_REGULAR): true { asort($this->__s, $flags); return true; }
    public function ksort(int $flags = SORT_REGULAR): true { ksort($this->__s, $flags); return true; }
    // spl_array_method parses the callback as "z": uasort() checks it.
    public function uasort($callback): true { uasort($this->__s, $callback); return true; }
    public function uksort($callback): true { uksort($this->__s, $callback); return true; }
    public function natsort(): true { natsort($this->__s); return true; }
    public function natcasesort(): true { natcasesort($this->__s); return true; }
    public function __serialize(): array { return [$this->__f & 0xffff, $this->__w ?? $this->__s, get_mangled_object_vars($this), $this->__i === 'ArrayIterator' ? null : $this->__i]; }
    public function __unserialize(array $data): void {
        self::__checkData($data);
        $this->__f = $data[0];
        if (is_object($data[1])) { __spl_deprecated('ArrayObject::__unserialize(): Using an object as a backing array for ArrayObject is deprecated, as it allows violating class constraints and invariants'); }
        self::__store($this, $data[1]);
        $this->__loadMembers($data[2]);
        if (isset($data[3])) {
            $c = $data[3];
            if (!class_exists($c) && !interface_exists($c)) {
                throw new UnexpectedValueException("Cannot deserialize ArrayObject with iterator class '$c'; no such class exists");
            }
            if (!is_a($c, 'Iterator', true)) {
                throw new UnexpectedValueException("Cannot deserialize ArrayObject with iterator class '$c'; this class does not implement the Iterator interface");
            }
            $this->__i = $c;
        }
    }
    // ArrayObject::__unserialize's checks.
    private static function __checkData(array $data): void {
        if (!isset($data[0]) || !array_key_exists(1, $data) || !isset($data[2]) || !is_int($data[0]) || !is_array($data[2])
                || (isset($data[3]) && !is_string($data[3]))) {
            throw new UnexpectedValueException('Incomplete or ill-typed serialization data');
        }
        if (!is_array($data[1]) && !is_object($data[1])) {
            throw new InvalidArgumentException('Passed variable is not an array or object');
        }
    }
    // object_properties_load: members keyed by mangled names.
    private function __loadMembers(array $m): void { __object_properties_load($this, $m); }
    public function serialize(): string {
        __serialize_init();
        try {
            $s = 'x:' . serialize($this->__f & 0x0100ffff);
            if ($this->__o !== $this) { $s .= serialize($this->__o ?? $this->__w ?? $this->__s) . ';'; }
            $s .= 'm:' . serialize(get_mangled_object_vars($this));
        } finally { __serialize_destroy(); }
        return $s;
    }
    public function __debugInfo(): array { return __spl_array_debug_info($this); }
    public function unserialize(string $data): void {
        [$f, $s, $m] = __spl_array_unserialize($data, 'ArrayObject::unserialize');
        if ($f === null) { return; }
        if (is_object($s)) { __spl_deprecated('ArrayObject::unserialize(): Using an object as a backing array for ' . ($this instanceof ArrayIterator ? 'ArrayIterator' : 'ArrayObject') . ' is deprecated, as it allows violating class constraints and invariants'); $s = get_object_vars($s); }
        $this->__f = $f; if ($s !== null) { $this->__s = $s; }
        $this->__loadMembers($m);
    }
}


// ext/spl/spl_dllist.c. The list is nodes keyed by id: `__v` the values,
// `__nx` / `__pv` the links (a missing link is NULL). A node the traverse
// pointer still holds after it left the list has no value and no links, as
// php's element kept alive by its refcount.
class SplDoublyLinkedList implements Iterator, Countable, ArrayAccess, Serializable {
    const IT_MODE_LIFO = 2; const IT_MODE_FIFO = 0; const IT_MODE_DELETE = 1; const IT_MODE_KEEP = 0;
    private $__v = []; private $__nx = []; private $__pv = [];
    private $__h = null; private $__t = null; private $__n = 0; private $__id = 0;
    private $__tp = null; private $__pos = 0;
    // null until first used: spl_dllist_object_new_ex's SplStack/SplQueue bits.
    private $__fl = null;
    private function __flags(): int {
        return $this->__fl ??= ($this instanceof SplStack ? 6 : ($this instanceof SplQueue ? 4 : 0));
    }
    private function __push($v): void {
        $id = ++$this->__id;
        $this->__v[$id] = $v;
        if ($this->__t !== null) { $this->__pv[$id] = $this->__t; $this->__nx[$this->__t] = $id; } else { $this->__h = $id; }
        $this->__t = $id; $this->__n++;
    }
    private function __drop($id): void { unset($this->__v[$id], $this->__nx[$id], $this->__pv[$id]); }
    private function __pop() {
        $t = $this->__t;
        $p = $this->__pv[$t] ?? null;
        if ($p !== null) { unset($this->__nx[$p]); } else { $this->__h = null; }
        $this->__t = $p; $this->__n--;
        $v = $this->__v[$t]; $this->__drop($t);
        return $v;
    }
    private function __shift() {
        $h = $this->__h;
        $nx = $this->__nx[$h] ?? null;
        if ($nx !== null) { unset($this->__pv[$nx]); } else { $this->__t = null; }
        $this->__h = $nx; $this->__n--;
        $v = $this->__v[$h]; $this->__drop($h);
        return $v;
    }
    // spl_ptr_llist_offset
    private function __offset(int $i) {
        $back = ($this->__flags() & 2) !== 0;
        $c = $back ? $this->__t : $this->__h;
        for ($p = 0; $c !== null && $p < $i; $p++) { $c = $back ? ($this->__pv[$c] ?? null) : ($this->__nx[$c] ?? null); }
        return $c;
    }
    private static function __index($fn, $i): int { return __zpp_arg("SplDoublyLinkedList::$fn", 1, 'index', 'int', $i); }
    private static function __range($fn): never { throw new OutOfRangeException("SplDoublyLinkedList::$fn(): Argument #1 (\$index) is out of range"); }
    public function add(int $index, mixed $value): void {
        if ($index < 0 || $index > $this->__n) { self::__range('add'); }
        if ($index === $this->__n) { $this->__push($value); return; }
        $e = $this->__offset($index);
        $id = ++$this->__id;
        $this->__v[$id] = $value;
        $p = $this->__pv[$e] ?? null;
        $this->__nx[$id] = $e;
        if ($p === null) { $this->__h = $id; } else { $this->__pv[$id] = $p; $this->__nx[$p] = $id; }
        $this->__pv[$e] = $id;
        $this->__n++;
    }
    public function pop(): mixed { if ($this->__n === 0) { throw new RuntimeException("Can't pop from an empty datastructure"); } return $this->__pop(); }
    public function shift(): mixed { if ($this->__n === 0) { throw new RuntimeException("Can't shift from an empty datastructure"); } return $this->__shift(); }
    public function push(mixed $value): void { $this->__push($value); }
    public function unshift(mixed $value): void {
        $id = ++$this->__id;
        $this->__v[$id] = $value;
        if ($this->__h !== null) { $this->__nx[$id] = $this->__h; $this->__pv[$this->__h] = $id; } else { $this->__t = $id; }
        $this->__h = $id; $this->__n++;
    }
    public function top(): mixed { if ($this->__t === null) { throw new RuntimeException("Can't peek at an empty datastructure"); } return $this->__v[$this->__t]; }
    public function bottom(): mixed { if ($this->__h === null) { throw new RuntimeException("Can't peek at an empty datastructure"); } return $this->__v[$this->__h]; }
    private function __list(): array { $a = []; for ($c = $this->__h; $c !== null; $c = $this->__nx[$c] ?? null) { $a[] = $this->__v[$c]; } return $a; }
    public function __debugInfo(): array {
        return __spl_user_props($this) + ["\0SplDoublyLinkedList\0flags" => $this->__flags(), "\0SplDoublyLinkedList\0dllist" => $this->__list()];
    }
    public function count(): int { return $this->__n; }
    public function isEmpty(): bool { return $this->__n === 0; }
    public function setIteratorMode(int $mode): int {
        $f = $this->__flags();
        if (($f & 4) && ($f & 2) !== ($mode & 2)) { throw new RuntimeException("Iterators' LIFO/FIFO modes for SplStack/SplQueue objects are frozen"); }
        return $this->__fl = ($mode & 3) | ($f & 4);
    }
    public function getIteratorMode(): int { return $this->__flags(); }
    public function offsetExists($index): bool { $index = self::__index('offsetExists', $index); return $index >= 0 && $index < $this->__n; }
    public function offsetGet($index): mixed {
        $index = self::__index('offsetGet', $index);
        if ($index < 0 || $index >= $this->__n) { self::__range('offsetGet'); }
        return $this->__v[$this->__offset($index)];
    }
    public function offsetSet($index, mixed $value): void {
        if ($index === null) { $this->__push($value); return; }
        $index = __zpp_arg('SplDoublyLinkedList::offsetSet', 1, 'index', '?int', $index);
        if ($index < 0 || $index >= $this->__n) { self::__range('offsetSet'); }
        $e = $this->__offset($index);
        // The old value is released after the new one is in place.
        $old = $this->__v[$e]; $this->__v[$e] = $value; unset($old);
    }
    public function offsetUnset($index): void {
        $index = self::__index('offsetUnset', $index);
        if ($index < 0 || $index >= $this->__n) { self::__range('offsetUnset'); }
        $e = $this->__offset($index);
        $p = $this->__pv[$e] ?? null; $x = $this->__nx[$e] ?? null;
        if ($p !== null) { if ($x !== null) { $this->__nx[$p] = $x; } else { unset($this->__nx[$p]); } }
        if ($x !== null) { if ($p !== null) { $this->__pv[$x] = $p; } else { unset($this->__pv[$x]); } }
        if ($e === $this->__h) { $this->__h = $x; }
        if ($e === $this->__t) { $this->__t = $p; }
        $this->__n--;
        if ($this->__tp === $e) { $this->__tp = null; }
        // The value is released while the element is unlinked (a destructor
        // may use the list).
        $old = $this->__v[$e]; $this->__drop($e); unset($old);
    }
    public function rewind(): void {
        if ($this->__flags() & 2) { $this->__pos = $this->__n - 1; $this->__tp = $this->__t; }
        else { $this->__pos = 0; $this->__tp = $this->__h; }
    }
    public function current(): mixed { return $this->__tp === null ? null : ($this->__v[$this->__tp] ?? null); }
    public function key(): int { return $this->__pos; }
    // spl_dllist_it_helper_move_forward
    private function __forward(int $f): void {
        $old = $this->__tp;
        if ($old === null) { return; }
        if ($f & 2) {
            $this->__tp = $this->__pv[$old] ?? null; $this->__pos--;
            if (($f & 1) && $this->__n > 0) { $this->__pop(); }
        } else {
            $this->__tp = $this->__nx[$old] ?? null;
            if ($f & 1) { if ($this->__n > 0) { $this->__shift(); } } else { $this->__pos++; }
        }
    }
    public function prev(): void { $this->__forward($this->__flags() ^ 2); }
    public function next(): void { $this->__forward($this->__flags()); }
    public function valid(): bool { return $this->__tp !== null; }
    public function unserialize(string $data): void {
        if ($data === '') { return; }
        while ($this->__n > 0) { $this->__pop(); }
        $n = strlen($data);
        __unserialize_init();
        try {
            [$ok, $flags, $p] = __unserialize_value($data, 0, 'SplDoublyLinkedList::unserialize');
            if ($ok && is_int($flags)) {
                $this->__fl = $flags;
                while (($data[$p] ?? '') === ':') {
                    $p++;
                    [$ok, $v, $p] = __unserialize_value($data, $p, 'SplDoublyLinkedList::unserialize');
                    if (!$ok) { break; }
                    $this->__push($v);
                }
                if ($ok && $p >= $n) { return; }
            }
        } finally { __unserialize_destroy(); }
        throw new UnexpectedValueException("Error at offset $p of $n bytes");
    }
    public function serialize(): string {
        __serialize_init();
        try {
            $s = serialize($this->__flags());
            for ($c = $this->__h; $c !== null; $c = $this->__nx[$c] ?? null) { $s .= ':' . serialize($this->__v[$c]); }
        } finally { __serialize_destroy(); }
        return $s;
    }
    public function __serialize(): array { return [$this->__flags(), $this->__list(), get_mangled_object_vars($this)]; }
    public function __unserialize(array $data): void {
        if (!isset($data[0], $data[1], $data[2]) || !is_int($data[0]) || !is_array($data[1]) || !is_array($data[2])) {
            throw new UnexpectedValueException('Incomplete or ill-typed serialization data');
        }
        $this->__fl = $data[0];
        foreach ($data[1] as $v) { $this->__push($v); }
        __object_properties_load($this, $data[2]);
    }
}

class SplQueue extends SplDoublyLinkedList {
    public function enqueue(mixed $value): void { $this->push($value); }
    public function dequeue(): mixed { return $this->shift(); }
}

class SplStack extends SplDoublyLinkedList {}

// php_spl.c: the default autoloader. The extension list lives in a
// static (php's spl_autoload_extensions global, per request).
function spl_autoload_extensions(?string $file_extensions = null): string {
    static $exts = null;
    if ($file_extensions !== null) { $exts = $file_extensions; }
    return $exts ?? '.inc,.php';
}
function spl_autoload(string $class, ?string $file_extensions = null): void {
    $exts = $file_extensions ?? spl_autoload_extensions();
    $lc = strtolower(($class[0] ?? '') === '\\' ? substr($class, 1) : $class);
    while ($exts !== '') {
        $c = strpos($exts, ',');
        $ext = $c === false ? $exts : substr($exts, 0, $c);
        $exts = $c === false ? '' : substr($exts, $c + 1);
        // php_stream_open_for_zend_ex(USE_PATH): silent when missing; a
        // file already included is not run again.
        $file = stream_resolve_include_path(str_replace('\\', '/', $lc . $ext));
        if ($file !== false && !in_array($file, get_included_files(), true)) {
            include $file;
            if (class_exists($lc, false) || interface_exists($lc, false) || trait_exists($lc, false)) { return; }
        }
    }
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

function iterator_apply($iterator, callable $callback, $args = null) {
    $callback = __spl_fcc($callback);
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
    // The properties an old serialization left become the elements.
    public function __wakeup(): void {
        if ($this->__size === 0) {
            $d = [];
            foreach (__spl_user_props($this) as $v) { $d[] = $v; }
            foreach (array_keys(get_object_vars($this)) as $k) { unset($this->$k); }
            $this->__d = $d; $this->__size = count($d);
        }
    }
    public function __serialize(): array { return $this->__d; }
    public function __unserialize(array $data): void {
        // ZVAL_COPY_DEREF: the elements' values, not references.
        $d = [];
        foreach ($data as $v) { $d[] = $v; }
        $this->__d = $d; $this->__size = count($data);
    }
}