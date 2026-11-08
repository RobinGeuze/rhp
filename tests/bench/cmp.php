<?php
class V { public $a; public $b; function __construct($a, $b) { $this->a = $a; $this->b = $b; } }
$x = new V(1, "x"); $y = new V(1, "x"); $z = new V(2, "y"); $n = 0;
for ($i = 0; $i < 3000000; $i++) { if ($x == $y) $n++; if ($x == $z) $n--; if ($x === $x) $n++; }
echo $n, "\n";
