<?php
// var_dump's "&": a reference held by more than one slot.
$x = 1;
$a = [&$x, 2];
var_dump($a);
unset($x);
var_dump($a);            // the last holder: no marker

function local() {
    $v = 'v';
    $arr = ['k' => &$v, 'n' => null];
    var_dump($arr);
    $o = new stdClass;
    $o->p = &$v;
    $o->q = 3;
    var_dump($o);
    unset($v);
    var_dump($o, $arr);
}
local();

// foreach by reference leaves the last element shared with $e.
$list = [1, 2, 3];
foreach ($list as &$e) {}
var_dump($list);
// php's array functions copy a reference only one slot holds as its value.
var_dump(array_values($list), array_slice($list, 1), array_reverse($list));
var_dump(array_merge($list, ['x' => &$e]), array_diff_key($list, [1 => 0]));
var_dump(array_intersect_key($list, [0 => 0, 2 => 0]), array_unique($list));
unset($e);
var_dump($list, array_values($list));

// nested containers
$inner = [1];
$outer = ['a' => &$inner, 'b' => [&$inner]];
var_dump($outer);

// closures: by-reference captures are shared, static variables are not
$cap = 5;
$f = function () use (&$cap) { static $s = 0; return $cap + $s; };
var_dump($f);
$g = function () use ($cap) { return $cap; };
var_dump($g);

// __debugInfo() returning references
class D {
    public $p = 1;
    public function __debugInfo() { return ['p' => &$this->p, 'c' => 2]; }
}
var_dump(new D);

// an enum case behind a reference
enum E { case A; }
$c = E::A;
var_dump([&$c]);

// foreach by reference over an element or a property: the loop's own
// reference to it goes away when the loop ends.
$nest = ['in' => [1, 2], 'x' => 0];
foreach ($nest['in'] as &$n) {}
unset($n);
$obj = new stdClass;
$obj->list = [1];
foreach ($obj->list as &$n) { break; }
unset($n);
var_dump($nest, $obj);

// spread copies like the array functions do
$s = 1;
$src = [&$s];
unset($s);
var_dump([...$src]);

// (array) of an object keeps the references its properties share
$shared = 1;
$po = new stdClass;
$po->r = &$shared;
$po->v = 2;
var_dump((array) $po);

// break 2 out of nested by-reference foreach loops frees both temporaries
$nested = ['a' => [1, 2]];
foreach ($nested['a'] as &$e1) {
    foreach ($nested['a'] as &$e2) {
        break 2;
    }
}
unset($e1, $e2);
var_dump($nested);
