<?php
class L { function __invoke() {} function m() {} static function s() {} }
$l = new L;
$a = $l(...); $b = $l(...); $c = (new L)(...);
var_dump($a == $b, $a == $c, $a === $b);
var_dump($l->m(...) == $l->m(...), $l->m(...) == $l(...), L::s(...) == L::s(...));
var_dump(strlen(...) == strlen(...), strlen(...) == strtolower(...));
$f = function() {}; $g = function() {};
var_dump($f == $g, $f == $f, Closure::fromCallable([$l, 'm']) == $l->m(...));
$x = 1; $u1 = function() use ($x) {}; 
var_dump($u1 == $u1->bindTo(null));
var_dump($a < $b, $a <=> $b);
var_dump(Closure::fromCallable($f) == $f);
var_dump(Closure::fromCallable([$l, '__invoke']) == $a, Closure::fromCallable($l) == $a, $l->__invoke(...) == $a);
