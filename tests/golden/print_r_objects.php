<?php
// print_r of objects: class header, nested blocks, cycles (*RECURSION*), enums.
class P { public $a = 1; public $arr = [1, [2]]; public $o; }
enum S: string { case H = 'h'; }
enum U { case X; }
$p = new P; $p->o = new stdClass; $p->o->k = 'v';
print_r($p); echo "\n";
$c = new stdClass; $c->self = $c; print_r($c); echo "\n";
print_r(S::H); echo "\n"; print_r(U::X); echo "\n";
print_r([new stdClass, 'x' => $p->o]); echo "\n";
echo print_r(new stdClass, true), "|\n";
