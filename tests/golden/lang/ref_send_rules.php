<?php
// Arguments for by-reference parameters of callees known only at run
// time, string callables, reference destructuring and foreach targets.

set_error_handler(function ($no, $msg, $file, $line) {
    echo "E$no: $msg (line $line)\n";
    return true;
});

function inc(&$x) { $x++; return $x; }
function val() { return 5; }
function &refg() { global $g; return $g; }

class C {
    public $p = 1;
    public static $s = [];
    public function m(&$a, $b = null) { $a = "m:$b"; }
    public static function sm(&$a) { $a = 'sm'; }
}

// a string callable naming a user function binds by reference
$f = 'inc';
$n = 1;
$f($n);
var_dump($n);
$f2 = '\inc';
$f2($n);
var_dump($n);

// method / static callee: a temporary throws, a call result notices
$o = new C;
try { $o->m(1); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { C::sm(PHP_INT_MAX); } catch (Error $e) { echo $e->getMessage(), "\n"; }
$o->m(val(), 'x');
$g = 'g0';
$o->m(refg(), 'y');
var_dump($g);
$m = 'm';
$o->$m($o->p, 'z');
var_dump($o->p);
try { $o->m($o?->p); } catch (Error $e) { echo $e->getMessage(), "\n"; }

// a reference assignment as the argument passes the reference
$q = 1;
$f($r =& $q);
var_dump($q, $r);

// reference destructuring into properties, static properties, nested keys
$src = ['a' => 1, 'b' => [2, 3]];
['a' => &$o->p, 'b' => [1 => &C::$s[0]]] = $src;
$o->p = 10;
C::$s[0] = 30;
var_dump($src);
$pair = [[0], [7]];
[$v1, [&$v2]] = $pair;
$v2 = 8;
var_dump($v1, $pair);

// foreach into a property key and an element value
$t = [];
foreach (['x' => 1, 'y' => 2] as $o->p => $t[]) {}
var_dump($o->p, $t);
foreach ([[1, 2], [3, 4]] as $i => [$t['a'], $t['b']]) {}
var_dump($i, $t);
$data = [[1], [2]];
foreach ($data as [&$e]) { $e *= 10; }
unset($e);
var_dump($data);

// typed by-reference and variadic parameters coerce in place
function as_int(int &$x, int ...$rest) { var_dump($rest); }
$s = "42";
as_int($s, "1", 2.0);
var_dump($s);
function refs(string &...$xs) {}
$a1 = 1; $a2 = 2.5;
refs($a1, $a2);
var_dump($a1, $a2);

// fn& returns the referenced variable
$base = 1;
$get = fn&(&$v) => $v;
$alias =& $get($base);
$alias = 99;
var_dump($base);

// spread with named keys into by-reference parameters
function two($a, &$b) { $b .= '!'; }
$args = ['b' => 'hi', 'a' => 0];
two(...$args);
var_dump($args);

// a reference into an ArrayObject offset is a slot
$ao = new ArrayObject([]);
$w = 'w';
$ao['k'] = &$w;
$w = 'changed';
var_dump($ao['k']);
