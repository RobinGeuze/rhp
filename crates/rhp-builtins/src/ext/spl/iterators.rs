//! SPL iterators and data structures (php-src `ext/spl/spl_iterators.c`,
//! `spl_heap.c`, `spl_observer.c`) in PHP source. `RecursiveIteratorIterator`
//! is a port of php's lazy per-level state machine (RS_START / RS_TEST /
//! RS_SELF / RS_CHILD / RS_NEXT, with the overridable hooks); the heaps use
//! php's exact sift-up / sift-down so equal elements come out in php's order.

/// The PHP source, spliced into the SPL prelude with `concat!`.
macro_rules! spl_iterators_prelude {
    () => {
        r#"
interface SplObserver { public function update(SplSubject $subject): void; }
interface SplSubject {
    public function attach(SplObserver $observer): void;
    public function detach(SplObserver $observer): void;
    public function notify(): void;
}

class SplObjectStorage implements Countable, SeekableIterator, ArrayAccess {
    private $__o = []; private $__v = []; private $__k = []; private $__p = 0; private $__pos = -1; private $__rw = false;
    public function attach(object $object, mixed $info = null): void {
        __spl_deprecated('Method SplObjectStorage::attach() is deprecated since 8.5, use method SplObjectStorage::offsetSet() instead');
        $this->offsetSet($object, $info);
    }
    public function detach(object $object): void {
        __spl_deprecated('Method SplObjectStorage::detach() is deprecated since 8.5, use method SplObjectStorage::offsetUnset() instead');
        $this->offsetUnset($object);
    }
    public function contains(object $object): bool {
        __spl_deprecated('Method SplObjectStorage::contains() is deprecated since 8.5, use method SplObjectStorage::offsetExists() instead');
        return $this->offsetExists($object);
    }
    public function addAll(SplObjectStorage $storage): int {
        foreach ($storage->__o as $h => $o) { $this->__o[$h] = $o; $this->__v[$h] = $storage->__v[$h]; }
        return count($this->__o);
    }
    public function removeAll(SplObjectStorage $storage): int {
        foreach ($storage->__o as $o) { $h = $this->getHash($o); unset($this->__o[$h], $this->__v[$h]); }
        return count($this->__o);
    }
    public function removeAllExcept(SplObjectStorage $storage): int {
        foreach ($this->__o as $h => $o) { if (!$storage->offsetExists($o)) { unset($this->__o[$h], $this->__v[$h]); } }
        return count($this->__o);
    }
    // The storage's own position starts at the first element, as php's
    // HashTable cursor does, until the first explicit rewind().
    private function __curHash() {
        if (!$this->__rw) { $this->rewind(); }
        return $this->__pos >= 0 ? ($this->__k[$this->__pos] ?? null) : null;
    }
    public function getInfo(): mixed { $h = $this->__curHash(); return ($h === null || !array_key_exists($h, $this->__o)) ? null : $this->__v[$h]; }
    public function setInfo(mixed $info): void { $h = $this->__curHash(); if ($h !== null && array_key_exists($h, $this->__o)) { $this->__v[$h] = $info; } }
    public function count(int $mode = COUNT_NORMAL): int {
        if ($mode === COUNT_RECURSIVE) { $n = count($this->__o); foreach ($this->__o as $o) { $n += count((array) $o, COUNT_RECURSIVE) * 0; } return $n; }
        return count($this->__o);
    }
    public function rewind(): void { $this->__rw = true; $this->__k = array_keys($this->__o); $this->__p = 0; $this->__pos = $this->__k ? 0 : -1; }
    public function valid(): bool { return $this->__curHash() !== null; }
    public function key(): int { return $this->__p; }
    public function current(): object {
        $h = $this->__curHash();
        if ($h === null || !array_key_exists($h, $this->__o)) { throw new RuntimeException('Called current() on invalid iterator'); }
        return $this->__o[$h];
    }
    private function __fwd(): void { if (!$this->__rw) { $this->rewind(); } if ($this->__pos >= 0) { $this->__pos++; if ($this->__pos >= count($this->__k)) { $this->__pos = -1; } } }
    public function next(): void { $this->__fwd(); $this->__p++; }
    public function seek(int $offset): void {
        if ($offset < 0 || $offset >= count($this->__o)) { throw new OutOfBoundsException("Seek position $offset is out of range"); }
        if ($offset === 0 || ($offset < $this->__p && $this->__p - $offset > $offset)) { $this->rewind(); }
        while ($offset > $this->__p) { $this->__fwd(); $this->__p++; }
        while ($offset < $this->__p) { if ($this->__pos > 0) { $this->__pos--; } elseif ($this->__pos === 0) { $this->__pos = -1; } $this->__p--; }
    }
    public function offsetExists($object): bool { return is_object($object) && array_key_exists($this->getHash($object), $this->__o); }
    public function offsetGet($object): mixed {
        $h = $this->getHash($object);
        if (!array_key_exists($h, $this->__o)) { throw new UnexpectedValueException('Object not found'); }
        return $this->__v[$h];
    }
    public function offsetSet(mixed $object, mixed $info = null): void {
        if (!is_object($object)) { throw new TypeError('SplObjectStorage::offsetSet(): Argument #1 ($object) must be of type object, ' . get_debug_type($object) . ' given'); }
        $h = $this->getHash($object); $this->__o[$h] = $object; $this->__v[$h] = $info;
    }
    public function offsetUnset($object): void { $h = $this->getHash($object); unset($this->__o[$h], $this->__v[$h]); }
    public function getHash(object $object): string { return spl_object_hash($object); }
    public function __serialize(): array {
        $s = [];
        foreach ($this->__o as $h => $o) { $s[] = $o; $s[] = $this->__v[$h]; }
        return [$s, get_mangled_object_vars($this)];
    }
    public function __unserialize(array $data): void {
        if (!isset($data[0], $data[1]) || !is_array($data[0]) || !is_array($data[1]) || count($data[0]) % 2 !== 0) {
            throw new UnexpectedValueException('Incomplete or ill-typed serialization data');
        }
        $s = array_values($data[0]);
        for ($i = 0; $i < count($s); $i += 2) {
            if (!is_object($s[$i])) { throw new UnexpectedValueException('Non-object key'); }
            $this->offsetSet($s[$i], $s[$i + 1]);
        }
        // Member names come mangled ("\0*\0a", "\0Class\0a"), as
        // get_mangled_object_vars() / __serialize() wrote them.
        foreach ($data[1] as $k => $v) {
            $k = (string)$k;
            if ($k !== '' && $k[0] === "\0") {
                $p = strpos($k, "\0", 1); $c = substr($k, 1, $p - 1); $n = substr($k, $p + 1);
                \Closure::bind(function () use ($n, $v) { $this->$n = $v; }, $this, $c === '*' ? static::class : $c)();
            } else { $this->$k = $v; }
        }
    }
    public function __debugInfo() {
        $st = [];
        foreach ($this->__o as $h => $o) { $st[] = ['obj' => $o, 'inf' => $this->__v[$h]]; }
        return ["\0SplObjectStorage\0storage" => $st];
    }
}

abstract class SplHeap implements Iterator, Countable {
    private $__h = []; private $__n = 0; private $__corrupt = false;
    abstract protected function compare(mixed $value1, mixed $value2): int;
    protected function __cmp($a, $b): int { return $this->compare($a, $b); }
    private function __check(): void {
        if ($this->__corrupt) { throw new RuntimeException('Heap is corrupted, heap properties are no longer ensured.'); }
    }
    public function insert(mixed $value): true {
        $this->__check();
        try {
            for ($pos = $this->__n; $pos > 0 && $this->__cmp($this->__h[intdiv($pos - 1, 2)], $value) < 0; $pos = intdiv($pos - 1, 2)) {
                $this->__h[$pos] = $this->__h[intdiv($pos - 1, 2)];
            }
        } catch (Throwable $e) { $this->__corrupt = true; $this->__h[$pos] = $value; $this->__n++; throw $e; }
        $this->__n++;
        $this->__h[$pos] = $value;
        return true;
    }
    protected function __deleteTop(string $what) {
        if ($this->__n === 0) { throw new RuntimeException("Can't $what from an empty heap"); }
        $this->__check();
        $limit = intdiv($this->__n - 1, 2);
        $top = $this->__h[0];
        $bottom = $this->__h[--$this->__n];
        $parent = 0;
        try {
            for ($parent = 0; $parent < $limit; $parent = $child) {
                $child = $parent * 2 + 1;
                if ($child !== $this->__n && $this->__cmp($this->__h[$child + 1], $this->__h[$child]) > 0) { $child++; }
                if ($this->__cmp($bottom, $this->__h[$child]) < 0) { $this->__h[$parent] = $this->__h[$child]; } else { break; }
            }
        } catch (Throwable $e) { $this->__corrupt = true; $this->__h[$parent] = $bottom; unset($this->__h[$this->__n]); throw $e; }
        // the vacated last slot still holds `bottom` during the sift (as in C)
        if ($parent !== $this->__n) { $this->__h[$parent] = $bottom; }
        unset($this->__h[$this->__n]);
        return $top;
    }
    public function extract(): mixed { return $this->__deleteTop('extract'); }
    public function top(): mixed {
        if ($this->__n === 0) { throw new RuntimeException("Can't peek at an empty heap"); }
        $this->__check();
        return $this->__h[0];
    }
    public function count(): int { return $this->__n; }
    public function isEmpty(): bool { return $this->__n === 0; }
    public function rewind(): void {}
    public function current(): mixed { return $this->__n === 0 ? null : $this->__h[0]; }
    public function key(): int { return $this->__n - 1; }
    public function next(): void { if ($this->__n > 0) { $this->__deleteTop('extract'); } }
    public function valid(): bool { return $this->__n > 0; }
    public function recoverFromCorruption(): true { $this->__corrupt = false; return true; }
    public function isCorrupted(): bool { return $this->__corrupt; }
    public function __debugInfo() {
        $h = [];
        for ($i = 0; $i < $this->__n; $i++) { $h[] = $this->__h[$i]; }
        return ["\0SplHeap\0flags" => 0, "\0SplHeap\0isCorrupted" => $this->__corrupt, "\0SplHeap\0heap" => $h];
    }
}
class SplMinHeap extends SplHeap {
    protected function compare(mixed $value1, mixed $value2): int { return $value2 <=> $value1; }
}
class SplMaxHeap extends SplHeap {
    protected function compare(mixed $value1, mixed $value2): int { return $value1 <=> $value2; }
}

class SplPriorityQueue implements Iterator, Countable {
    const EXTR_BOTH = 3; const EXTR_PRIORITY = 2; const EXTR_DATA = 1;
    private $__h = []; private $__n = 0; private $__flags = 1; private $__corrupt = false;
    public function compare(mixed $priority1, mixed $priority2): int { return $priority1 <=> $priority2; }
    private function __cmp($a, $b): int { return $this->compare($a['priority'], $b['priority']); }
    private function __out($e) {
        switch ($this->__flags) {
            case 3: return $e;
            case 2: return $e['priority'];
            default: return $e['data'];
        }
    }
    private function __check(): void {
        if ($this->__corrupt) { throw new RuntimeException('Heap is corrupted, heap properties are no longer ensured.'); }
    }
    public function insert(mixed $value, mixed $priority) {
        $this->__check();
        $elem = ['data' => $value, 'priority' => $priority];
        for ($pos = $this->__n; $pos > 0 && $this->__cmp($this->__h[intdiv($pos - 1, 2)], $elem) < 0; $pos = intdiv($pos - 1, 2)) {
            $this->__h[$pos] = $this->__h[intdiv($pos - 1, 2)];
        }
        $this->__n++;
        $this->__h[$pos] = $elem;
        return true;
    }
    private function __deleteTop(string $what) {
        if ($this->__n === 0) { throw new RuntimeException("Can't $what from an empty heap"); }
        $this->__check();
        $limit = intdiv($this->__n - 1, 2);
        $top = $this->__h[0];
        $bottom = $this->__h[--$this->__n];
        for ($parent = 0; $parent < $limit; $parent = $child) {
            $child = $parent * 2 + 1;
            if ($child !== $this->__n && $this->__cmp($this->__h[$child + 1], $this->__h[$child]) > 0) { $child++; }
            if ($this->__cmp($bottom, $this->__h[$child]) < 0) { $this->__h[$parent] = $this->__h[$child]; } else { break; }
        }
        if ($parent !== $this->__n) { $this->__h[$parent] = $bottom; }
        unset($this->__h[$this->__n]);
        return $top;
    }
    public function setExtractFlags(int $flags): int {
        $flags &= 3;
        if ($flags === 0) { throw new RuntimeException('Must specify at least one extract flag'); }
        $this->__flags = $flags;
        return $flags;
    }
    public function getExtractFlags(): int { return $this->__flags; }
    public function extract(): mixed { return $this->__out($this->__deleteTop('extract')); }
    public function top(): mixed {
        if ($this->__n === 0) { throw new RuntimeException("Can't peek at an empty heap"); }
        $this->__check();
        return $this->__out($this->__h[0]);
    }
    public function count(): int { return $this->__n; }
    public function isEmpty(): bool { return $this->__n === 0; }
    public function rewind(): void {}
    public function current(): mixed { return $this->__n === 0 ? null : $this->__out($this->__h[0]); }
    public function key(): int { return $this->__n - 1; }
    public function next(): void { if ($this->__n > 0) { $this->__deleteTop('extract'); } }
    public function valid(): bool { return $this->__n > 0; }
    public function recoverFromCorruption(): true { $this->__corrupt = false; return true; }
    public function isCorrupted(): bool { return $this->__corrupt; }
    public function __debugInfo() {
        $h = [];
        for ($i = 0; $i < $this->__n; $i++) { $h[] = $this->__h[$i]; }
        return ["\0SplPriorityQueue\0flags" => $this->__flags, "\0SplPriorityQueue\0isCorrupted" => $this->__corrupt, "\0SplPriorityQueue\0heap" => $h];
    }
}

class RecursiveIteratorIterator implements OuterIterator {
    const LEAVES_ONLY = 0; const SELF_FIRST = 1; const CHILD_FIRST = 2; const CATCH_GET_CHILD = 16;
    private $__its = []; private $__st = []; private $__level = 0; private $__mode = 0; private $__flags = 0;
    private $__maxDepth = -1; private $__inIter = false;
    public function __construct(Traversable $iterator, int $mode = self::LEAVES_ONLY, int $flags = 0) {
        if ($iterator instanceof IteratorAggregate) { $iterator = $iterator->getIterator(); }
        if (!($iterator instanceof RecursiveIterator)) {
            throw new InvalidArgumentException('An instance of RecursiveIterator or IteratorAggregate creating it is required');
        }
        $this->__its = [$iterator]; $this->__st = [0]; $this->__level = 0;
        $this->__mode = $mode; $this->__flags = $flags;
    }
    // states: 0 = START, 1 = TEST, 2 = SELF, 3 = CHILD, 4 = NEXT
    private function __forward(): void {
        while (true) {
            $it = $this->__its[$this->__level];
            $st = $this->__st[$this->__level];
            if ($st === 4) { $it->next(); $st = 0; }
            if ($st === 0) {
                $level = $this->__level;
                $valid = $it->valid();
                // valid() re-entered next()/rewind() and moved the iterator
                // on (php 8.5.12, php-src efbf473): that call already advanced.
                if ($this->__level !== $level || ($this->__its[$level] ?? null) !== $it) { return; }
                $st = $valid ? 1 : 5;
            }
            if ($st === 1) {
                if ($this->callHasChildren()) {
                    if ($this->__maxDepth === -1 || $this->__maxDepth > $this->__level) {
                        $this->__st[$this->__level] = $this->__mode === self::SELF_FIRST ? 2 : 3;
                        continue;
                    }
                    if ($this->__mode === self::LEAVES_ONLY) { $this->__st[$this->__level] = 4; continue; }
                }
                $this->nextElement();
                $this->__st[$this->__level] = 4;
                return;
            }
            if ($st === 2) {
                if ($this->__mode === self::SELF_FIRST || $this->__mode === self::CHILD_FIRST) { $this->nextElement(); }
                $this->__st[$this->__level] = $this->__mode === self::SELF_FIRST ? 3 : 4;
                return;
            }
            if ($st === 3) {
                try {
                    $child = $this->callGetChildren();
                } catch (Throwable $e) {
                    if (!($this->__flags & self::CATCH_GET_CHILD)) { $this->__st[$this->__level] = 4; throw $e; }
                    $this->__st[$this->__level] = 4;
                    continue;
                }
                if (!($child instanceof RecursiveIterator)) {
                    throw new UnexpectedValueException('Objects returned by RecursiveIterator::getChildren() must implement RecursiveIterator');
                }
                $this->__st[$this->__level] = $this->__mode === self::CHILD_FIRST ? 2 : 4;
                $this->__level++;
                $this->__its[$this->__level] = $child; $this->__st[$this->__level] = 0;
                $child->rewind();
                $this->beginChildren();
                continue;
            }
            // 5: this level is exhausted
            if ($this->__level > 0) {
                $this->endChildren();
                unset($this->__its[$this->__level], $this->__st[$this->__level]);
                $this->__level--;
            } else {
                return;
            }
        }
    }
    public function rewind(): void {
        while ($this->__level > 0) {
            unset($this->__its[$this->__level], $this->__st[$this->__level]);
            $this->__level--;
            $this->endChildren();
        }
        $this->__st[0] = 0;
        $this->__its[0]->rewind();
        if (!$this->__inIter) { $this->beginIteration(); }
        $this->__inIter = true;
        $this->__forward();
    }
    public function valid(): bool {
        for ($l = $this->__level; $l >= 0; $l--) {
            if ($this->__its[$l]->valid()) { return true; }
        }
        if ($this->__inIter) { $this->__inIter = false; $this->endIteration(); }
        return false;
    }
    // SimpleXML's internal iterator (a subclass overriding nothing) has no
    // current element past the end, where its current() and key() throw.
    public function key(): mixed { $it = $this->__its[$this->__level]; return self::__sxeDone($it) ? null : $it->key(); }
    public function current(): mixed { $it = $this->__its[$this->__level]; return self::__sxeDone($it) ? null : $it->current(); }
    private static function __sxeDone($it): bool { return ($it::class === 'SimpleXMLElement' || $it::class === 'SimpleXMLIterator') && !$it->valid(); }
    public function next(): void { $this->__forward(); }
    public function getDepth(): int { return $this->__level; }
    public function getSubIterator(?int $level = null): ?RecursiveIterator {
        $l = $level ?? $this->__level;
        return ($l < 0 || $l > $this->__level) ? null : $this->__its[$l];
    }
    public function getInnerIterator(): RecursiveIterator { return $this->__its[$this->__level]; }
    public function beginIteration() {}
    public function endIteration() {}
    public function callHasChildren() { return $this->__its[$this->__level]->hasChildren(); }
    public function callGetChildren() { return $this->__its[$this->__level]->getChildren(); }
    public function beginChildren() {}
    public function endChildren() {}
    public function nextElement() {}
    public function setMaxDepth(int $maxDepth = -1): void {
        if ($maxDepth < -1) { throw new OutOfRangeException('RecursiveIteratorIterator::setMaxDepth(): Argument #1 ($maxDepth) must be greater than or equal to -1'); }
        if ($maxDepth > PHP_INT_MAX) { $maxDepth = PHP_INT_MAX; }
        $this->__maxDepth = $maxDepth;
    }
    public function getMaxDepth(): int|false { return $this->__maxDepth === -1 ? false : $this->__maxDepth; }
    // php forwards unknown methods to the current sub-iterator.
    public function __call($name, $args) { return call_user_func_array([$this->__its[$this->__level], $name], $args); }
}

class IteratorIterator implements OuterIterator {
    protected $__it;
    public function __construct(Traversable $iterator, ?string $class = null) {
        $this->__it = ($iterator instanceof IteratorAggregate) ? $iterator->getIterator() : $iterator;
    }
    public function getInnerIterator(): ?Iterator { return $this->__it; }
    public function rewind(): void { $this->__it->rewind(); }
    public function valid(): bool { return $this->__it->valid(); }
    public function current(): mixed { return $this->__it->current(); }
    public function key(): mixed { return $this->__it->key(); }
    public function next(): void { $this->__it->next(); }
    // SPL forwards unknown calls (getFilename/getPathname/isDir/…) to the inner
    // iterator, so callers can treat a filter iterator like its SplFileInfo items.
    public function __call($name, $args) { return call_user_func_array([$this->__it, $name], $args); }
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
    public function getChildren(): ?RecursiveFilterIterator {
        return new static($this->__it->getChildren());
    }
}

class RecursiveCallbackFilterIterator extends CallbackFilterIterator implements RecursiveIterator {
    public function __construct(RecursiveIterator $iterator, callable $callback) {
        parent::__construct($iterator, $callback);
    }
    public function hasChildren(): bool { return $this->__it->hasChildren(); }
    public function getChildren(): RecursiveCallbackFilterIterator {
        return new static($this->__it->getChildren(), $this->__cb);
    }
}

class ParentIterator extends RecursiveFilterIterator {
    public function accept(): bool { return $this->__it->hasChildren(); }
}

class AppendIterator extends IteratorIterator {
    private $__a; private $__has = false; private $__cur = null; private $__key = null;
    public function __construct() { $this->__a = new ArrayIterator(); $this->__it = null; }
    private function __innerValid(): bool { return $this->__it !== null && $this->__it->valid(); }
    private function __free(): void { $this->__has = false; $this->__cur = null; $this->__key = null; }
    private function __nextIterator(): bool {
        $this->__free();
        $this->__it = null;
        if ($this->__a->valid()) {
            $this->__it = $this->__a->current();
            $this->__it->rewind();
            return true;
        }
        return false;
    }
    private function __fetch(): void {
        while (!$this->__innerValid()) {
            $this->__a->next();
            if (!$this->__nextIterator()) { return; }
        }
        $this->__cur = $this->__it->current(); $this->__key = $this->__it->key(); $this->__has = true;
    }
    public function append(Iterator $iterator): void {
        if ($this->__a->valid() && !$this->__innerValid()) {
            $this->__a->append($iterator);
            $this->__a->next();
        } else {
            $this->__a->append($iterator);
        }
        if ($this->__it === null || !$this->__innerValid()) {
            if (!$this->__a->valid()) { $this->__a->rewind(); }
            $this->__nextIterator();
            while ($this->__it !== $iterator && $this->__a->valid()) { $this->__a->next(); $this->__nextIterator(); }
            $this->__fetch();
        }
    }
    public function rewind(): void {
        $this->__a->rewind();
        if ($this->__nextIterator()) { $this->__fetch(); }
    }
    public function valid(): bool { return $this->__has; }
    public function current(): mixed { return $this->__cur; }
    public function key(): mixed { return $this->__key; }
    public function next(): void {
        if ($this->__innerValid()) { $this->__free(); $this->__it->next(); }
        $this->__fetch();
    }
    public function getIteratorIndex(): ?int { return $this->__a->key(); }
    public function getArrayIterator(): ArrayIterator { return $this->__a; }
    public function getInnerIterator(): ?Iterator { return $this->__it; }
}

class LimitIterator extends IteratorIterator {
    private $__off = 0; private $__cnt = -1; private $__pos = 0; private $__has = false; private $__cur = null; private $__key = null;
    public function __construct(Iterator $iterator, int $offset = 0, int $limit = -1) {
        if ($offset < 0) { throw new ValueError('LimitIterator::__construct(): Argument #2 ($offset) must be greater than or equal to 0'); }
        if ($limit < -1) { throw new ValueError('LimitIterator::__construct(): Argument #3 ($limit) must be greater than or equal to -1'); }
        parent::__construct($iterator);
        $this->__off = $offset; $this->__cnt = $limit;
    }
    private function __fetch(bool $check): bool {
        $this->__has = false; $this->__cur = null; $this->__key = null;
        if (!$check || $this->__it->valid()) {
            $this->__cur = $this->__it->current(); $this->__key = $this->__it->key(); $this->__has = true;
            return true;
        }
        return false;
    }
    private function __seek(int $pos): void {
        if ($pos < $this->__off) {
            throw new OutOfBoundsException("Cannot seek to $pos which is below the offset {$this->__off}");
        }
        if ($pos >= $this->__off + $this->__cnt && $this->__cnt != -1) {
            throw new OutOfBoundsException("Cannot seek to $pos which is behind offset {$this->__off} plus count {$this->__cnt}");
        }
        if ($pos != $this->__pos && $this->__it instanceof SeekableIterator) {
            $this->__it->seek($pos);
            $this->__pos = $pos;
            $this->__fetch(true);
        } else {
            if ($pos < $this->__pos) { $this->__has = false; $this->__pos = 0; $this->__it->rewind(); }
            while ($pos > $this->__pos && $this->__it->valid()) { $this->__has = false; $this->__it->next(); $this->__pos++; }
            if ($this->__it->valid()) { $this->__fetch(true); }
        }
    }
    public function rewind(): void {
        $this->__has = false; $this->__pos = 0; $this->__it->rewind();
        $this->__seek($this->__off);
    }
    public function valid(): bool {
        return ($this->__cnt == -1 || $this->__pos < $this->__off + $this->__cnt) && $this->__has;
    }
    public function current(): mixed { return $this->__cur; }
    public function key(): mixed { return $this->__key; }
    public function next(): void {
        $this->__has = false; $this->__cur = null; $this->__key = null;
        $this->__it->next(); $this->__pos++;
        if ($this->__cnt == -1 || $this->__pos < $this->__off + $this->__cnt) { $this->__fetch(true); }
    }
    public function seek(int $offset): int { $this->__seek($offset); return $this->__pos; }
    public function getPosition(): int { return $this->__pos; }
}

class NoRewindIterator extends IteratorIterator {
    public function __construct(Iterator $iterator) { parent::__construct($iterator); }
    public function rewind(): void {}
    public function valid(): bool { return $this->__it->valid(); }
    public function key(): mixed { return $this->__it->key(); }
    public function current(): mixed { return $this->__it->current(); }
    public function next(): void { $this->__it->next(); }
}

class InfiniteIterator extends IteratorIterator {
    public function __construct(Iterator $iterator) { parent::__construct($iterator); }
    public function next(): void {
        $this->__it->next();
        if (!$this->__it->valid()) { $this->__it->rewind(); }
    }
}

class EmptyIterator implements Iterator {
    public function current(): never { throw new BadMethodCallException('Accessing the value of an EmptyIterator'); }
    public function next(): void {}
    public function key(): never { throw new BadMethodCallException('Accessing the key of an EmptyIterator'); }
    public function valid(): bool { return false; }
    public function rewind(): void {}
}

class RegexIterator extends FilterIterator {
    const USE_KEY = 1; const INVERT_MATCH = 2;
    const MATCH = 0; const GET_MATCH = 1; const ALL_MATCHES = 2; const SPLIT = 3; const REPLACE = 4;
    public ?string $replacement = null;
    protected $__re; protected $__mode; protected $__rflags; protected $__pflags; protected $__usePflags;
    protected $__cur = null; protected $__key = null; protected $__has = false;
    public function __construct(Iterator $iterator, string $pattern, int $mode = self::MATCH, int $flags = 0, int $pregFlags = 0) {
        parent::__construct($iterator);
        if ($mode < 0 || $mode > 4) {
            throw new ValueError('RegexIterator::__construct(): Argument #3 ($mode) must be RegexIterator::MATCH, RegexIterator::GET_MATCH, RegexIterator::ALL_MATCHES, RegexIterator::SPLIT, or RegexIterator::REPLACE');
        }
        if (@preg_match($pattern, '') === false) {
            throw new InvalidArgumentException('Illegal regular expression');
        }
        $this->__re = $pattern; $this->__mode = $mode; $this->__rflags = $flags;
        $this->__pflags = $pregFlags; $this->__usePflags = func_num_args() >= 5;
    }
    private function __fetchAccept(): void {
        while ($this->__it->valid()) {
            $this->__cur = $this->__it->current(); $this->__key = $this->__it->key(); $this->__has = true;
            if ($this->accept()) { return; }
            $this->__it->next();
        }
        $this->__has = false; $this->__cur = null; $this->__key = null;
    }
    public function rewind(): void { $this->__it->rewind(); $this->__fetchAccept(); }
    public function next(): void { $this->__it->next(); $this->__fetchAccept(); }
    public function valid(): bool { return $this->__has; }
    public function current(): mixed { return $this->__cur; }
    public function key(): mixed { return $this->__key; }
    public function accept(): bool {
        if (!$this->__has) { return false; }
        if ($this->__rflags & self::USE_KEY) { $subject = (string) $this->__key; }
        else { if (is_array($this->__cur)) { return false; } $subject = (string) $this->__cur; }
        switch ($this->__mode) {
            case self::MATCH:
                $r = preg_match($this->__re, $subject) > 0;
                break;
            case self::GET_MATCH:
            case self::ALL_MATCHES:
                $m = [];
                $pf = $this->__usePflags ? $this->__pflags : 0;
                $n = $this->__mode === self::ALL_MATCHES ? preg_match_all($this->__re, $subject, $m, $pf ?: PREG_PATTERN_ORDER) : preg_match($this->__re, $subject, $m, $pf);
                $this->__cur = $m;
                $r = $n > 0;
                break;
            case self::SPLIT:
                $this->__cur = preg_split($this->__re, $subject, -1, $this->__usePflags ? $this->__pflags : 0);
                $r = count($this->__cur) > 1;
                break;
            default:
                $res = preg_replace($this->__re, (string) $this->replacement, $subject, -1, $count);
                if ($this->__rflags & self::USE_KEY) { $this->__key = $res; } else { $this->__cur = $res; }
                $r = $count > 0;
        }
        if ($this->__rflags & self::INVERT_MATCH) { return !$r; }
        return $r;
    }
    public function getMode(): int { return $this->__mode; }
    public function setMode(int $mode): void {
        if ($mode < 0 || $mode > 4) {
            throw new ValueError('RegexIterator::setMode(): Argument #1 ($mode) must be RegexIterator::MATCH, RegexIterator::GET_MATCH, RegexIterator::ALL_MATCHES, RegexIterator::SPLIT, or RegexIterator::REPLACE');
        }
        $this->__mode = $mode;
    }
    public function getFlags(): int { return $this->__rflags; }
    public function setFlags(int $flags): void { $this->__rflags = $flags; }
    public function getPregFlags(): int { return $this->__usePflags ? $this->__pflags : 0; }
    public function setPregFlags(int $pregFlags): void { $this->__pflags = $pregFlags; $this->__usePflags = true; }
    public function getRegex(): string { return $this->__re; }
}

class RecursiveRegexIterator extends RegexIterator implements RecursiveIterator {
    public function __construct(RecursiveIterator $iterator, string $pattern, int $mode = self::MATCH, int $flags = 0, int $pregFlags = 0) {
        parent::__construct($iterator, $pattern, $mode, $flags, $pregFlags);
    }
    public function accept(): bool {
        if (!$this->__has) { return false; }
        if (is_array($this->__cur)) { return count($this->__cur) > 0; }
        return parent::accept();
    }
    public function hasChildren(): bool { return $this->__it->hasChildren(); }
    public function getChildren(): RecursiveRegexIterator {
        return new static($this->__it->getChildren(), $this->__re, $this->__mode, $this->__rflags, $this->__pflags);
    }
}

class CachingIterator extends IteratorIterator implements ArrayAccess, Countable, Stringable {
    const CALL_TOSTRING = 1; const CATCH_GET_CHILD = 16; const TOSTRING_USE_KEY = 2;
    const TOSTRING_USE_CURRENT = 4; const TOSTRING_USE_INNER = 8; const FULL_CACHE = 256;
    protected $__cflags = 1; protected $__valid = false; protected $__cur = null; protected $__key = null;
    protected $__str = null; protected $__cache = []; protected $__children = null;
    public function __construct(Iterator $iterator, int $flags = self::CALL_TOSTRING) {
        self::__checkFlags($flags, static::class === 'RecursiveCachingIterator' ? 'RecursiveCachingIterator::__construct' : 'CachingIterator::__construct', 2);
        parent::__construct($iterator);
        $this->__cflags = $flags;
    }
    protected static function __checkFlags(int $flags, string $fn, int $arg): void {
        $n = 0;
        foreach ([1, 2, 4, 8] as $f) { if ($flags & $f) { $n++; } }
        if ($n > 1) {
            throw new ValueError("$fn(): Argument #$arg (\$flags) must contain only one of CachingIterator::CALL_TOSTRING, CachingIterator::TOSTRING_USE_KEY, CachingIterator::TOSTRING_USE_CURRENT, or CachingIterator::TOSTRING_USE_INNER");
        }
    }
    protected function __cnext(): void {
        $this->__cur = null; $this->__key = null; $this->__str = null;
        if ($this->__it->valid()) {
            $this->__cur = $this->__it->current(); $this->__key = $this->__it->key();
            $this->__valid = true;
            if ($this->__cflags & self::FULL_CACHE) { $this->__cache[$this->__key] = $this->__cur; }
            $this->__children = null;
            if ($this instanceof RecursiveCachingIterator) {
                try {
                    if ($this->__it->hasChildren()) {
                        $this->__children = new RecursiveCachingIterator($this->__it->getChildren(), $this->__cflags);
                    }
                } catch (Throwable $e) {
                    if (!($this->__cflags & self::CATCH_GET_CHILD)) { throw $e; }
                    $this->__children = null;
                }
            }
            $this->__str = null;
            if ($this->__cflags & self::TOSTRING_USE_INNER) { $this->__str = (string) $this->__it; }
            elseif ($this->__cflags & self::CALL_TOSTRING) { $this->__str = is_array($this->__cur) ? 'Array' : (string) $this->__cur; }
            $this->__it->next();
        } else {
            $this->__valid = false;
        }
    }
    public function rewind(): void { $this->__it->rewind(); $this->__cache = []; $this->__cnext(); }
    public function valid(): bool { return $this->__valid; }
    public function current(): mixed { return $this->__cur; }
    public function key(): mixed { return $this->__key; }
    public function next(): void { $this->__cnext(); }
    public function hasNext(): bool { return $this->__it->valid(); }
    public function __toString(): string {
        if (!($this->__cflags & (self::CALL_TOSTRING | self::TOSTRING_USE_KEY | self::TOSTRING_USE_CURRENT | self::TOSTRING_USE_INNER))) {
            throw new BadMethodCallException('CachingIterator does not fetch string value (see CachingIterator::__construct)');
        }
        if ($this->__cflags & self::TOSTRING_USE_KEY) { return (string) $this->__key; }
        if ($this->__cflags & self::TOSTRING_USE_CURRENT) { return (string) $this->__cur; }
        return $this->__str ?? '';
    }
    public function getFlags(): int { return $this->__cflags; }
    public function setFlags(int $flags): void {
        self::__checkFlags($flags, 'CachingIterator::setFlags', 1);
        if (($this->__cflags & self::CALL_TOSTRING) && !($flags & self::CALL_TOSTRING)) {
            throw new InvalidArgumentException('Unsetting flag CALL_TO_STRING is not possible');
        }
        if (($this->__cflags & self::TOSTRING_USE_INNER) && !($flags & self::TOSTRING_USE_INNER)) {
            throw new InvalidArgumentException('Unsetting flag TOSTRING_USE_INNER is not possible');
        }
        if (($flags & self::FULL_CACHE) && !($this->__cflags & self::FULL_CACHE)) { $this->__cache = []; }
        $this->__cflags = $flags;
    }
    private function __full(): void {
        if (!($this->__cflags & self::FULL_CACHE)) {
            throw new BadMethodCallException(static::class . ' does not use a full cache (see CachingIterator::__construct)');
        }
    }
    public function offsetGet($key): mixed {
        $this->__full();
        if (!array_key_exists($key, $this->__cache)) { trigger_error("Undefined array key \"$key\"", E_USER_WARNING); return null; }
        return $this->__cache[$key];
    }
    public function offsetSet($key, mixed $value): void { $this->__full(); $this->__cache[$key] = $value; }
    public function offsetUnset($key): void { $this->__full(); unset($this->__cache[$key]); }
    public function offsetExists($key): bool { $this->__full(); return isset($this->__cache[$key]); }
    public function getCache(): array { $this->__full(); return $this->__cache; }
    public function count(): int { $this->__full(); return count($this->__cache); }
}

class RecursiveCachingIterator extends CachingIterator implements RecursiveIterator {
    public function __construct(Iterator $iterator, int $flags = self::CALL_TOSTRING) { parent::__construct($iterator, $flags); }
    public function hasChildren(): bool { return $this->__children !== null; }
    public function getChildren(): ?RecursiveCachingIterator { return $this->__children; }
}

class RecursiveArrayIterator extends ArrayIterator implements RecursiveIterator {
    const CHILD_ARRAYS_ONLY = 4;
    public function hasChildren(): bool {
        $c = $this->current();
        return is_array($c) || (is_object($c) && !($this->getFlags() & self::CHILD_ARRAYS_ONLY));
    }
    public function getChildren(): ?RecursiveArrayIterator {
        $c = $this->current();
        if ($c instanceof static) { return $c; }
        return new static($c, $this->getFlags());
    }
}

class RecursiveTreeIterator extends RecursiveIteratorIterator {
    const BYPASS_CURRENT = 4; const BYPASS_KEY = 8;
    const PREFIX_LEFT = 0; const PREFIX_MID_HAS_NEXT = 1; const PREFIX_MID_LAST = 2;
    const PREFIX_END_HAS_NEXT = 3; const PREFIX_END_LAST = 4; const PREFIX_RIGHT = 5;
    private $__rflags; private $__prefix = ['', '| ', '  ', '|-', '\\-', '']; private $__postfix = '';
    public function __construct($iterator, int $flags = self::BYPASS_KEY, int $cachingIteratorFlags = CachingIterator::CATCH_GET_CHILD, int $mode = self::SELF_FIRST) {
        if ($iterator instanceof IteratorAggregate) { $iterator = $iterator->getIterator(); }
        parent::__construct(new RecursiveCachingIterator($iterator, $cachingIteratorFlags), $mode, $flags);
        $this->__rflags = $flags;
    }
    public function getPrefix(): string {
        $s = $this->__prefix[0];
        $depth = $this->getDepth();
        for ($l = 0; $l < $depth; $l++) {
            $s .= $this->getSubIterator($l)->hasNext() ? $this->__prefix[1] : $this->__prefix[2];
        }
        $s .= $this->getSubIterator($depth)->hasNext() ? $this->__prefix[3] : $this->__prefix[4];
        return $s . $this->__prefix[5];
    }
    public function setPrefixPart(int $part, string $value): void {
        if ($part < 0 || $part > 5) { throw new OutOfRangeException('RecursiveTreeIterator::setPrefixPart(): Argument #1 ($part) must be a RecursiveTreeIterator::PREFIX_* constant'); }
        $this->__prefix[$part] = $value;
    }
    public function getEntry(): ?string {
        $c = parent::current();
        if (is_array($c)) { return 'Array'; }
        return (string) $c;
    }
    public function setPostfix(string $postfix): void { $this->__postfix = $postfix; }
    public function getPostfix(): string { return $this->__postfix; }
    public function current(): mixed {
        if ($this->__rflags & self::BYPASS_CURRENT) { return parent::current(); }
        $e = $this->getEntry();
        if ($e === null) { return null; }
        return $this->getPrefix() . $e . $this->getPostfix();
    }
    public function key(): mixed {
        $k = parent::key();
        if ($this->__rflags & self::BYPASS_KEY) { return $k; }
        return $this->getPrefix() . $k . $this->getPostfix();
    }
}

class MultipleIterator implements Iterator {
    const MIT_NEED_ANY = 0; const MIT_NEED_ALL = 1; const MIT_KEYS_NUMERIC = 0; const MIT_KEYS_ASSOC = 2;
    private $__its = []; private $__infos = []; private $__flags;
    public function __construct(int $flags = MultipleIterator::MIT_NEED_ALL | MultipleIterator::MIT_KEYS_NUMERIC) { $this->__flags = $flags; }
    public function getFlags(): int { return $this->__flags; }
    public function setFlags(int $flags): void { $this->__flags = $flags; }
    public function attachIterator(Iterator $iterator, string|int|null $info = null): void {
        $h = spl_object_id($iterator);
        if ($info !== null) {
            foreach ($this->__infos as $oh => $i) {
                if ($oh !== $h && $i === $info) { throw new InvalidArgumentException('Key duplication error'); }
            }
        }
        $this->__its[$h] = $iterator; $this->__infos[$h] = $info;
    }
    public function detachIterator(Iterator $iterator): void { $h = spl_object_id($iterator); unset($this->__its[$h], $this->__infos[$h]); }
    public function containsIterator(Iterator $iterator): bool { return isset($this->__its[spl_object_id($iterator)]); }
    public function countIterators(): int { return count($this->__its); }
    public function rewind(): void { foreach ($this->__its as $it) { $it->rewind(); } }
    public function valid(): bool {
        if (!$this->__its) { return false; }
        $all = (bool) ($this->__flags & self::MIT_NEED_ALL);
        foreach ($this->__its as $it) {
            if ($it->valid()) { if (!$all) { return true; } }
            elseif ($all) { return false; }
        }
        return $all;
    }
    public function next(): void { foreach ($this->__its as $it) { $it->next(); } }
    private function __collect(bool $cur): array {
        $out = [];
        foreach ($this->__its as $h => $it) {
            if ($it->valid()) { $v = $cur ? $it->current() : $it->key(); }
            elseif ($this->__flags & self::MIT_NEED_ALL) {
                throw new RuntimeException('Called ' . ($cur ? 'current' : 'key') . '() with non valid sub iterator');
            } else { $v = null; }
            if ($this->__flags & self::MIT_KEYS_ASSOC) {
                $info = $this->__infos[$h];
                if ($info === null) { throw new InvalidArgumentException('Sub-Iterator is associated with NULL'); }
                $out[$info] = $v;
            } else { $out[] = $v; }
        }
        return $out;
    }
    public function current(): array { return $this->__collect(true); }
    public function key(): array { return $this->__collect(false); }
    public function __debugInfo() {
        $st = [];
        foreach ($this->__its as $h => $it) { $st[] = ['obj' => $it, 'inf' => $this->__infos[$h]]; }
        return ["\0SplObjectStorage\0storage" => $st];
    }
}
"#
    };
}

pub(crate) use spl_iterators_prelude;
