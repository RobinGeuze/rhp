<?php
class P { public $a = 1; public int $b = 2; private $c = 3; function get() { return $this->c; } }
$p = new P; $t = 0;
for ($i = 0; $i < 5000000; $i++) { $t += $p->a + $p->b; $p->a = $i & 7; }
for ($i = 0; $i < 1000000; $i++) { $t += $p->get(); }
echo $t, "\n";
