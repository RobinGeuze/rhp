<?php
// Declared properties keep their slot (declaration order, parent first);
// dynamic ones follow in insertion order.
class A { public $a = 1; private $p = 'A'; protected $b = 2; }
#[AllowDynamicProperties]
class B extends A { public $c = 3; private $p = 'B'; public int $t; public $a = 10; }

$o = new B;
var_dump($o);
unset($o->a);
$o->d = 4;
$o->a = 5;          // back in its slot, ahead of $d
$o->t = 7;          // uninitialized typed slot filled
print_r($o);
echo json_encode($o), "\n";
var_dump(array_keys((array) $o));
var_dump(get_object_vars($o));
echo serialize($o), "\n";
var_export($o); echo "\n";

// Unset and re-add of a dynamic property moves it last.
$o->e = 6;
unset($o->d);
$o->d = 8;
foreach ($o as $k => $v) echo "$k=$v ";
echo "\n";

// foreach while unsetting / re-adding declared properties.
$o = new B;
foreach ($o as $k => $v) {
    echo "$k ";
    if ($k === 'a') { unset($o->c); }
    if ($k === 'b') { $o->c = 33; $o->x = 1; }
}
echo "\n";
var_dump($o);

// References in slots, clone, comparison.
$o = new B;
$r = &$o->c;
$r = 99;
$c = clone $o;
$r = 100;
var_dump($o->c, $c->c, $o == $c);
unset($o->c, $c->c);
var_dump($o == $c, count((array) $o));

// stdClass: everything dynamic.
$s = new stdClass;
$s->z = 1; $s->y = 2; unset($s->z); $s->z = 3;
var_dump($s, (array) $s);
$u = unserialize(serialize($o));
var_dump($u);
