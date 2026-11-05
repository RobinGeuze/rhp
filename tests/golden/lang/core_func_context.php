<?php
// func_get_args()/func_num_args()/func_get_arg() and get_called_class():
// variadics, named arguments, modified parameters, includes from functions.

function v($a, ...$rest) {
    $a = 'changed';
    var_dump(func_num_args(), func_get_args());
    try {
        func_get_arg(-1);
    } catch (ValueError $e) {
        echo $e->getMessage(), "\n";
    }
    var_dump(func_get_arg(2));
}
v(1, 2, 3);

function named($a, $b = 'B', $c = 'C') {
    var_dump(func_num_args(), func_get_args());
}
named(1, c: 3);

function extra($a) {
    var_dump(func_get_args());
}
extra(1, 2, [3]);

class P {
    public static function who() { return get_called_class(); }
    public function me() { return get_called_class(); }
    public static function fwd() { return forward_static_call([P::class, 'who']); }
    public static function plain() { return call_user_func([P::class, 'who']); }
    public static function closure() { return (function () { return get_called_class(); })(); }
}
class C extends P {}
var_dump(C::who(), (new C)->me(), C::fwd(), C::plain(), C::closure());
var_dump(forward_static_call_array('strtoupper', ['abc']));
function pad($s, $n = 1, $side = 'r') { return "$s/$n/$side"; }
var_dump(forward_static_call_array('pad', ['x', 'side' => 'l']));

try {
    forward_static_call('strlen', 'x');
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}

$f = function () { return func_get_args(); };
var_dump($f(1, 2));
