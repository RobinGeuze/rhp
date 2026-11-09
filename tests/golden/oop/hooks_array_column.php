<?php
class A { public int $n = 1; public $v { get => $this->n * 10; } public $b = 5 { get => $this->b + 1; } private $p { get => 'priv'; } public $w { set { echo "set\n"; } } }
$rows = [new A, new A];
var_dump(array_column($rows, 'v'), array_column($rows, 'b'), array_column($rows, 'p'), array_column($rows, 'n', 'v'));
try { var_dump(array_column($rows, 'w')); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
