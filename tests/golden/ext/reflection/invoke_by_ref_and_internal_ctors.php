<?php
ob_start(fn ($s) => str_replace(__DIR__, 'D', $s));
// zend_call_function's by-reference check through ReflectionMethod.
class A
{
    public function __construct(&$s) { echo "A::__construct\n"; }
    public static function bump(&$n, $m = 0) { $n++; return $n; }
}
$m = new ReflectionMethod('A', 'bump');
var_dump($m->invoke(null, 1));
var_dump($m->invokeArgs(null, [5, 6]));
$x = 1;
var_dump($m->invokeArgs(null, [&$x]), $x);
$o = (new ReflectionClass('A'))->newInstanceWithoutConstructor();
(new ReflectionMethod('A', '__construct'))->invoke($o, 'v');

// A readonly property of a built-in class initialized from its own scope.
$d = (new ReflectionClass('Directory'))->newInstanceWithoutConstructor();
(new ReflectionProperty('Directory', 'path'))->setValue($d, '/x');
var_dump($d);
try {
    (new ReflectionProperty('Directory', 'path'))->setValue($d, '/y');
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
try {
    $d->path = '/z';
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}

// Classes whose get_constructor refuses `new` have no constructor frame.
function make($c)
{
    try {
        new $c();
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n", $e->getTraceAsString(), "\n";
    }
}
foreach (['CurlHandle', 'XMLParser', 'PDORow', 'Socket', 'DeflateContext'] as $c) {
    make($c);
}
