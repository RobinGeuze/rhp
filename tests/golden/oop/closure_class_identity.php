<?php
// A closure is an object of class Closure everywhere a class is asked for.
interface I {}
class A { public $v = 5; }
$c = function ($x = 1) { return $this->v + $x; };
$n = 'Closure';
$f = strlen(...);
var_dump($c::class, $f::class, (fn() => 1)::class);
var_dump($c instanceof $n, $c instanceof $c, $f instanceof $c, $c instanceof Closure);
$i = 'I';
var_dump($c instanceof $i, $c instanceof Traversable, (new A) instanceof $c);
var_dump(class_implements($c), class_parents($c), class_uses($c));
var_dump(get_class_methods('Closure'), get_class_methods($c));

// Closure's own methods as callables (zend_closure_get_method).
var_dump(is_callable([$c, 'call'], false, $name), $name);
var_dump(is_callable([$c, 'bindTo']), is_callable([$c, 'bind']), is_callable([$c, 'nope']), is_callable([$c, '__construct']));
var_dump(call_user_func([$c, 'call'], new A, 2));
var_dump(call_user_func([$c, 'bindTo'], new A)());
var_dump(call_user_func_array([$c, 'call'], [new A, 4]));
var_dump(call_user_func_array([$c, 'call'], ['newThis' => new A, 'x' => 9]));
$m = [$c, 'call'];
var_dump($m(new A));
$g = $c->call(...);
var_dump($g(new A, 7));
$h = $c->bindTo(...);
var_dump($h(new A)());
var_dump(array_map([$c, 'call'], [new A]));
var_dump(call_user_func([$c, 'fromCallable'], 'strlen')('abc'));
try {
    call_user_func([$c, 'nope']);
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
try {
    [$c, 'nope']();
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}

// Closure::call's arguments go before the closure it was called on:
// object handles come back in php's order.
(function () { return 5; })->call(new A);
$a = new stdClass; $b = new stdClass;
var_dump(spl_object_id($a), spl_object_id($b));
unset($a, $b);
set_error_handler(function ($no, $msg) { echo "warning: $msg\n"; return true; });
(function () { return $this; })->call(new stdClass);
var_dump(spl_object_id(new stdClass));
