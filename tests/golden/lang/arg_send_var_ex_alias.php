<?php
// SEND_VAR_EX sends a variable before the later arguments run, also when
// they write it through a reference, a global, a by-ref capture, eval,
// extract or a variable variable.
class O { function m($a, $b) { var_dump($a); } }
function w() { global $g; $g = 'w'; return 1; }
$o = new O;
// alias
$p = 1; $r = &$p; $o->m($p, $r = 'x');
unset($r);
// global
$g = 1; $o->m($g, w());
function inner($o) {
  $p = 1; $r = &$p; $o->m($p, $r = 'y');
  $q = 1; $f = function() use (&$q) { $q = 'cap'; return 1; }; $o->m($q, $f());
  $e = 1; $o->m($e, eval('$e = "ev"; return 1;'));
  $x = 1; $o->m($x, extract(['x' => 'ext']));
  $z = 1; $n = 'z'; $o->m($z, $$n = 'dyn');
  $s = 1; $o->m($s, ($t = 'a') . 'b');
}
inner($o);
$o2 = new class { function m($a, $b, $c) { var_dump($a); } };
function viaArg(&$x) { $x = 'byref'; return 1; }
function outer($o2) { $p = 1; $o2->m($p, 0, viaArg($p)); $k = 1; $o2->m($k, $k++, 0); }
outer($o2);
// The variable on the left of `= &` aliases too.
class P { public $x = 'o'; function set() { $this->x = 'm'; return 1; } }
function lhsRef($o2) {
    $a = 'o'; $b = &$a; $o2->m($b, $a = 'n', 0);
    $arr = ['o']; $e = &$arr[0]; $o2->m($e, $arr[0] = 'w', 0);
    $p = new P; $x = &$p->x; $o2->m($x, $p->set(), 0);
}
lhsRef($o2);
