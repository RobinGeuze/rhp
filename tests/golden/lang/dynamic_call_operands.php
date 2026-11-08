<?php
// A dynamic call reads its callee (and a method call its object and name)
// before the arguments run; by-reference parameters of a builtin or a
// first-class callable reached at run time bind the variables passed.
function a($x) { echo "a($x)\n"; } function b($x) { echo "b($x)\n"; }
$f = 'a'; $f($f = 'b');
$f = 'a'; $f(...[$f = 'b']);
class C { function a($x) { echo "C::a\n"; } function b($x) { echo "C::b\n"; } }
$o = new C; $m = 'a'; $o->$m($m = 'b');
$c = 'C'; $m = 'a'; (new $c)->$m($m = 'b');
$s = 'sort'; $arr = [3, 1, 2]; $s($arr, ...[]); var_dump($arr);
class D2 { function a($x) { echo "D::a\n"; } }
$o = new C; $o->a($o = new D2);
$o = new C; $o?->a($o = new D2);
$f = 'a'; $f($f);
class C2 { function m(&$x) { $x = 'm'; } static function s(&$x) { $x = 's'; } }
$o = new C2;
$f = $o->m(...); $a = 0; $f($a); var_dump($a);
$f = Closure::fromCallable([$o, 'm']); $a = 0; $f($a); var_dump($a);
$f = C2::s(...); $a = 0; $f($a); var_dump($a);
$f = Closure::fromCallable('C2::s'); $a = 0; $f($a); var_dump($a);
function g2(&$x) { $x = 'g'; }
$n = "g2"; $a = 0; $n(x: $a); var_dump($a);
$n = [$o, 'm']; $a = 0; $n(x: $a); var_dump($a);
$f = $o->m(...); $a = 0; $f(x: $a); var_dump($a);
$n = 'sort'; $a = [2, 1]; $n(array: $a); var_dump($a);
