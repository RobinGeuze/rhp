<?php
class A { public $x; const K = 3; function __construct($x) { $this->x = $x; } }
class B { public $p = 1; public $q = [1, 2]; }
$s = 0; $b = new B;
for ($i = 0; $i < 1000000; $i++) { $a = new A($i); $s += $a->x + A::K + PHP_INT_SIZE; $c = clone $b; $s += $c->p; }
echo $s, "\n";
