<?php
class Dict implements ArrayAccess, Countable, IteratorAggregate {
    private array $d = [];
    public function offsetExists($k): bool { return array_key_exists($k, $this->d); }
    public function offsetGet($k): mixed { return $this->d[$k] ?? null; }
    public function offsetSet($k, $v): void { if ($k === null) $this->d[] = $v; else $this->d[$k] = $v; }
    public function offsetUnset($k): void { unset($this->d[$k]); }
    public function count(): int { return count($this->d); }
    public function getIterator(): Iterator { return new ArrayIterator2($this->d); }
}
class ArrayIterator2 implements Iterator {
    private int $i = 0; private array $keys;
    public function __construct(private array $d) { $this->keys = array_keys($d); }
    public function rewind(): void { $this->i = 0; }
    public function valid(): bool { return $this->i < count($this->keys); }
    public function current(): mixed { return $this->d[$this->keys[$this->i]]; }
    public function key(): mixed { return $this->keys[$this->i]; }
    public function next(): void { $this->i++; }
}
$m = new Dict();
$m['a'] = 1; $m['b'] = 2; $m[] = 'z';
echo $m['a'], $m['b'], $m[0], "\n";
echo count($m), "\n";
var_dump(isset($m['a']), isset($m['x']));
unset($m['a']);
echo count($m), "\n";
foreach ($m as $k => $v) echo "$k=$v ";
echo "\n";
