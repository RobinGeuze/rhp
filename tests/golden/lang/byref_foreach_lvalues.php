<?php
// By-reference foreach over a property, and references bound to array
// elements and properties.
class C { public $items = [1, 2, 3]; function dbl() { foreach ($this->items as &$v) $v *= 2; } }
$c = new C(); $c->dbl(); echo implode(',', $c->items), "\n";
$a = []; $x = 5; $a['k'] =& $x; $x = 9; echo $a['k'], "\n";
$o = new C(); $v = 1; $o->items['n'] =& $v; $v = 7; echo $o->items['n'], "\n";
$ref =& $c->items[0]; $ref = 100; echo $c->items[0], "\n";
