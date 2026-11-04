<?php
// `??=` on every lvalue form: CV, array element (nested), property (nested,
// dynamic, magic), static property, ArrayAccess. The left side is fetched
// quietly, its sub-expressions are evaluated once, and the right side only
// runs when the current value is null.
function f() { echo "f() called\n"; return 2; }
function key_of($k) { echo "key_of($k)\n"; return $k; }

$x = 1; $x ??= f(); var_dump($x);
$y = null; var_dump($y ??= f()); var_dump($y);
var_dump($fresh ??= 'fresh');

$a = []; $a['x'] ??= 1; $a['x'] ??= 9;
$b = ['a' => ['b' => null]]; $b['a']['b'] ??= 3; $b['a']['b'] ??= f();
var_dump($a, $b);
var_dump($b['new']['deep'] ??= 100, $b['new']['deep'] ??= 200);

$i = 0; $k = []; $k[$i++] ??= 'x'; $k[$i++] ??= 'y'; var_dump($i, $k);
$m = []; $m[key_of('a')] ??= 1; $m[key_of('a')] ??= 2; var_dump($m);

$o = new stdClass; $o->p ??= 5; $o->p ??= f();
$o->q['r'] ??= 'deep'; $o->q['r'] ??= f();
$n = 'dyn'; $o->$n ??= 'D'; $o->{$n . '2'} ??= 'D2';
var_dump($o);

class C {
    public static $s;
    public static $t = [];
    public static function f() { self::$t['q'] ??= 4; static::$t['q'] ??= 5; }
}
C::$s ??= 7; C::$s ??= 8; C::f();
var_dump(C::$s, C::$t);

class AA implements ArrayAccess {
    public $d = ['has' => 1, 'nul' => null];
    function offsetExists($k): bool { echo "offsetExists($k)\n"; return array_key_exists($k, $this->d); }
    function offsetGet($k): mixed { echo "offsetGet($k)\n"; return $this->d[$k]; }
    function offsetSet($k, $v): void { echo "offsetSet($k)\n"; $this->d[$k] = $v; }
    function offsetUnset($k): void {}
}
$aa = new AA;
$aa['has'] ??= 5; $aa['no'] ??= 6; $aa['nul'] ??= 7;
var_dump($aa->d);

class Magic {
    private $hidden = [];
    function __isset($n) { echo "__isset($n)\n"; return $n === 'set'; }
    function __get($n) { echo "__get($n)\n"; return $n === 'set' ? 'S' : null; }
    function __set($n, $v) { echo "__set($n)\n"; }
}
$mg = new Magic;
$mg->hidden ??= 1; $mg->absent ??= 2; var_dump($mg->set ??= 3);

$s = 'str'; var_dump($s[1] ??= 'q');
