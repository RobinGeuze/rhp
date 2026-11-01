<?php
// `$o->$m(...)` / `C::$m(...)` (and the nullsafe `?->$m(...)`) with argument
// spread and named arguments.
class A {
    function f(...$a) { return count($a); }
    static function g($x, $y = 0) { return [$x, $y]; }
    private function priv($a, $b = 'B') { return "priv:$a:$b"; }
    function callPriv($m, $args) { return $this->$m(...$args); }
    function __call($n, $a) { return "__call:$n:" . json_encode($a); }
    static function __callStatic($n, $a) { return "__callStatic:$n:" . json_encode($a); }
    function named(int $a, int $b = 2, int ...$rest) { return [$a, $b, $rest]; }
}
$m = 'f';
echo (new A)->$m(...[1, 2]), "\n";
$n = 'g';
var_dump(A::$n(...[1, 2]), A::$n(y: 5, x: 1));
$o = null;
var_dump($o?->$m(...[1]), $o?->f(...[1]), $o?->$m(a: 1));
$a = new A;
var_dump($a?->$m(...[1, 2, 3]), $a?->f(...[4]));
var_dump($a->callPriv('priv', ['x']));
var_dump($a->callPriv('priv', ['b' => 'y', 'a' => 'z']));
$p = 'priv';
var_dump($a->$p(...['q']));
$u = 'nope';
var_dump($a->$u(...[1, 2]), $a->$u(1, k: 2));
var_dump(A::$u(...[1]), A::$u(z: 3));
$nm = 'named';
var_dump($a->$nm(1, ...[5, 6, 7]), $a->$nm(b: 9, a: 8));
$cl = function ($x, $y) { return $x + $y; };
$inv = '__invoke';
var_dump($cl->$inv(...[1, 2]));
try {
    $z = null;
    $z->$m(...[1]);
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
function gen() { yield 1; yield 2; }
$g = gen();
$cur = 'current';
var_dump($g->$cur(...[]));
enum E: string { case X = 'x'; }
$fr = 'from';
var_dump(E::$fr(...['x']));
class B extends A {
    function t() { $g = 'g'; return [parent::$g(...[3]), static::$g(x: 4)]; }
}
var_dump((new B)->t());
