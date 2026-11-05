<?php
// Default values against declared types: compile-time widening, the
// run-time check of constant-expression defaults, implicit nullable types.

const STR_FIVE = "5";
const INT_TWO = 2;

function weak_int(int $a = STR_FIVE) { var_dump($a); }
function to_float(float $a = INT_TWO) { var_dump($a); }
function lit_float(float $a = 3, int|float $b = 4, ?float $c = 5) { var_dump($a, $b, $c); }
weak_int();
to_float();
lit_float();

class K {
    const BAD = [1];
    const float F = 1;
    public float $p = 7;
    public static float $s = 8;
    public function m(string $s = self::BAD) { return $s; }
}
var_dump(K::F, (new K)->p, K::$s);
try {
    (new K)->m();
} catch (TypeError $e) {
    echo get_class($e), ": ", preg_replace('/called in .*/', 'called in X', $e->getMessage()), "\n";
}

interface I {}
interface J {}
class IJ implements I, J {}
// Compile-time deprecations name the file: include it under a handler.
set_error_handler(function ($no, $msg, $file, $line) {
    echo "Deprecated: $msg in ", basename($file), " on line $line\n";
    return true;
});
require __DIR__ . '/type_decl_defaults_implicit.inc';
restore_error_handler();
implicit();
implicit(new IJ);
foreach ((new ReflectionFunction('implicit'))->getParameters() as $p) {
    echo $p->getName(), ": ", $p->getType(), "\n";
}

function dnf(int|(I&J)|string|null $v) { return $v; }
echo (new ReflectionFunction('dnf'))->getParameters()[0]->getType(), "\n";
try {
    dnf([]);
} catch (TypeError $e) {
    echo preg_replace('/called in .*/', 'called in X', $e->getMessage()), "\n";
}

function only_null(null $n) { return $n; }
var_dump(only_null(null));
try {
    only_null(0);
} catch (TypeError $e) {
    echo preg_replace('/called in .*/', 'called in X', $e->getMessage()), "\n";
}

function unloaded(\Ns\Missing $m) {}
try {
    unloaded(new stdClass);
} catch (TypeError $e) {
    echo preg_replace('/called in .*/', 'called in X', $e->getMessage()), "\n";
}

// Reflection prints a float default with its fraction.
function fdef(float $a = 3, $b = 1.0, $c = -0.0, $d = 2.5) {}
foreach ((new ReflectionFunction('fdef'))->getParameters() as $p) {
    echo $p, "\n";
}
