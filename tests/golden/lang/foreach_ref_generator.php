<?php
// foreach by reference over by-reference generators, directly or through an
// IteratorAggregate whose getIterator() is one (nette/neon's Traverser).
class Items implements IteratorAggregate
{
    public $items = ['x', 'y'];
    public function &getIterator(): Generator
    {
        foreach ($this->items as &$i) {
            yield $i;
        }
    }
}
$a = new Items;
foreach ($a as &$v) {
    $v = strtoupper($v);
}
unset($v);
var_dump($a->items);

function &g()
{
    $x = 1;
    yield $x;
    var_dump($x);
}
foreach (g() as &$r) {
    var_dump($r);
    $r = 5;
}
unset($r);

function &kv()
{
    $d = ['a' => 1, 'b' => 2];
    foreach ($d as $k => &$e) {
        yield $k => $e;
    }
    unset($e);
    echo json_encode($d), "\n";
}
foreach (kv() as $key => &$val) {
    $val *= 10;
    echo "$key\n";
}
unset($val);
// by value: the generator's variables are untouched
foreach (kv() as $key => $val) {
    $val = 0;
}
// by-ref generator used by value elsewhere
$gen = g();
var_dump($gen->current());
$gen->next();

function plain() { yield 1; }
try {
    foreach (plain() as &$p) {}
} catch (Exception $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}

class It implements Iterator
{
    public function current(): mixed { return 1; }
    public function key(): mixed { return 0; }
    public function next(): void {}
    public function rewind(): void {}
    public function valid(): bool { return false; }
}
try {
    foreach (new It as &$p) {}
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}

// by reference over a temporary array (a call result)
function arr() { return [1, 2, 3]; }
foreach (arr() as &$z) {
    $z *= 2;
}
var_dump($z);
unset($z);

$closureGen = (function &() {
    $q = 'a';
    yield $q;
    echo "q=$q\n";
})();
foreach ($closureGen as &$w) {
    $w = 'b';
}
unset($w);
