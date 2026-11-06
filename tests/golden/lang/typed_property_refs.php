<?php
// References bound to typed properties check what is written through them.
class T {
    public int $i = 1;
    public ?string $s = null;
    public float $f = 0.5;
    public int|string $u = 0;
    public array $a = [];
    public static int $si = 3;
}

function show(callable $f) {
    try {
        $f();
    } catch (TypeError $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}

$t = new T;
$r = &$t->i;
$r = "42";
var_dump($t->i);
show(function () use (&$r) { $r = "abc"; });
show(function () use (&$r) { $r = 1.0; });
var_dump($t->i);
$r += 1;
var_dump($t->i);
show(function () use (&$r) { $r .= "x"; });
var_dump($t->i);

// A by-reference parameter bound to the property.
function setIt(&$x, $v) { $x = $v; }
show(function () use ($t) { setIt($t->i, []); });
setIt($t->i, "7");
var_dump($t->i);

// Two sources must agree on the coercion.
$t2 = new T;
$t2->s = "5";
$t2->u = "5";
$x = "5";
$t2->s = &$x;
$t2->u = &$x;
show(function () use (&$x) { $x = 5; });
var_dump($x);
show(function () use (&$x) { $x = 5.0; });
var_dump($x);

// unset() of the property removes it as a source.
unset($t2->s);
$x = 5;
var_dump($x);

// Freeing the object removes it too.
$o = new T;
$p = &$o->i;
show(function () use (&$p) { $p = "no"; });
unset($o);
$p = "yes";
var_dump($p);

// Static properties.
$sr = &T::$si;
show(function () use (&$sr) { $sr = []; });
$sr = "9";
var_dump(T::$si);

// list() and array elements by reference.
$t3 = new T;
$arr = [&$t3->f];
$arr[0] = 2;
var_dump($t3->f);
show(function () use (&$arr) { $arr[0] = "z"; });
[$q] = [10];
$m = [&$t3->i];
show(function () use (&$m) { $m[0] = null; });
var_dump($t3->i);

// Auto-initializing an array inside a typed reference.
$t4 = new T;
$t4->s = null;
$sref = &$t4->s;
show(function () use (&$sref) { $sref[] = 1; });
var_dump($sref);
$aref = &$t4->a;
$aref[] = 1;
var_dump($t4->a);

// strict_types follows the writing code.
$f = &$t4->f;
$f = 3;
var_dump($t4->f);

// Builtins writing by-reference arguments.
$t5 = new T;
show(function () use ($t5) { preg_match('/a/', 'a', $t5->i); });
var_dump($t5->i);
preg_match('/a/', 'a', $t5->a);
var_dump($t5->a);
settype($t5->f, "string");
var_dump($t5->f);

// catch assigns strictly.
$cr = &$t5->s;
try {
    try {
        throw new Exception("e");
    } catch (Exception $cr) {
        echo "unreachable\n";
    }
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
