<?php
// By-reference foreach follows the array (HashTable iterator positions).
$a = [1, 2, 3];
foreach ($a as $k => &$v) {
    echo "$k=$v ";
    if ($k == 0) { $copy = $a; unset($a[1]); $a[] = 4; }
}
unset($v);
echo "\n", implode(",", $a), " / ", implode(",", $copy), "\n";

// Reassigning the iterated variable restarts at the new array's pointer.
$a = [1, 2, 3];
$b = [10, 20, 30];
next($b);
$i = 0;
foreach ($a as &$v) {
    echo "$v ";
    if (++$i == 1) { $a = $b; }
    if ($i > 5) break;
}
unset($v);
echo "\n";

// Variable becomes a non-array during a by-reference loop.
$a = [1, 2];
foreach ($a as &$v) { echo "$v\n"; $a = 5; }
unset($v);

// foreach by value over an object sees properties added and removed.
#[AllowDynamicProperties]
class P { public $a = 1; protected $b = 2; private $c = 3; public $d = 4;
    function walk() { foreach ($this as $k => $v) { echo "$k=$v "; } echo "\n"; } }
$o = new P;
foreach ($o as $k => $v) {
    echo "$k=$v ";
    if ($k == 'a') { unset($o->d); $o->e = 5; }
}
echo "\n";
$o->walk();
// An unset declared property comes back at its slot; the iterator sees it.
$o = new P;
unset($o->d);
foreach ($o as $k => $v) {
    echo "$k=$v ";
    if ($k == 'a') { $o->d = 'back'; }
}
echo "\n";

// Destructors of the iterated temporary run when the loop is left.
class D { function __construct(public $n) {} function __destruct() { echo "~D{$this->n} "; } }
function f() {
    foreach ([new D(1)] as $x) { $x = null; return "ret "; }
}
echo f(), "\n";
foreach ([new D(2)] as $x) { $x = null; break; }
echo "after break\n";
foreach ([new D(3)] as $x) { $x = null; foreach ([new D(4)] as $y) { $y = null; continue 2; } }
echo "after continue 2\n";

// An Iterator in foreach holds an object handle while the loop runs.
class It implements Iterator {
    private $i = 0;
    function current(): mixed { return new stdClass; }
    function key(): mixed { return $this->i; }
    function next(): void { $this->i++; }
    function rewind(): void { $this->i = 0; }
    function valid(): bool { return $this->i < 2; }
}
foreach (new It as $o) { var_dump(spl_object_id($o)); }
var_dump(spl_object_id(new stdClass));

// Recursive arrays: identical() and in_array(strict) throw.
$r = [&$r];
try { var_dump(in_array([1], $r, true)); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { var_dump($r === [[1]]); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump($r === $r);

// A sort comparison that writes to the array being sorted: the sort is lost.
class S { function __toString(): string { $GLOBALS['s'][] = 'x'; return 'b'; } }
$s = ['c', new S, 'a'];
sort($s);
echo count($s), "\n";
