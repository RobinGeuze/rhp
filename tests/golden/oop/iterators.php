<?php
class Coll implements Iterator {
    private int $pos = 0;
    public function __construct(private array $items) {}
    public function rewind(): void { $this->pos = 0; }
    public function valid(): bool { return $this->pos < count($this->items); }
    public function current(): mixed { return $this->items[$this->pos]; }
    public function key(): mixed { return $this->pos; }
    public function next(): void { $this->pos++; }
}
$c = new Coll([10, 20, 30]);
foreach ($c as $k => $v) echo "$k:$v ";
echo "\n";
foreach ($c as $v) echo $v;
echo "\n";

class Bag implements IteratorAggregate {
    public function __construct(private array $data) {}
    public function getIterator(): Iterator { return new Coll(array_values($this->data)); }
}
foreach (new Bag([5, 15, 25]) as $v) echo $v, " ";
echo "\n";

class G implements IteratorAggregate {
    public function getIterator(): Generator { yield 'a'; yield 'b'; yield 'c'; }
}
foreach (new G() as $k => $v) echo "$k=$v ";
echo "\n";

class Point { public int $x = 1; public int $y = 2; private int $hidden = 9; }
foreach (new Point() as $k => $v) echo "$k=$v ";
echo "\n";

var_dump((new Coll([1])) instanceof Traversable);
var_dump((new Bag([])) instanceof Traversable);
