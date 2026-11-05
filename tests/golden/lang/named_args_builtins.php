<?php
// Named arguments to builtin functions: placement from arginfo, stub
// defaults for skipped parameters, by-reference parameters by name, and the
// errors php raises.
function t($f) {
    try {
        var_dump($f());
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}

var_dump(preg_match('/(b)/', 'abc', flags: PREG_OFFSET_CAPTURE, matches: $m), $m);
t(fn() => strlen(str: 'x'));
t(fn() => strlen('x', string: 'y'));
t(fn() => sprintf('%s', x: 1));
t(fn() => array_keys([1], strict: true));
t(fn() => array_keys(strict: true));
t(fn() => str_replace(search: 'a', replace: 'b'));
var_dump(strlen(...)(string: 'abc'));
var_dump(call_user_func('str_pad', 'a', 4, pad_type: STR_PAD_LEFT, pad_string: '-'));
var_dump(call_user_func_array('str_pad', ['a', 4, 'pad_type' => STR_PAD_BOTH]));
var_dump(json_encode(value: [1], flags: JSON_PRETTY_PRINT));
var_dump(array_fill(start_index: 0, count: 2, value: 'v'));
var_dump(htmlspecialchars('<a>', double_encode: false));
var_dump(number_format(1234.5, thousands_separator: ' '));
var_dump(in_array(needle: '1', haystack: [1], strict: true));
$arr = [3, 1, 2];
sort(array: $arr);
var_dump($arr);
var_dump(round(1.55, precision: 1));
var_dump(intdiv(num2: 2, num1: 7));
$s = 'x';
var_dump(settype(type: 'int', var: $s), $s);
t(fn() => array_merge([1], a: [2]));

// by-reference parameters of user functions by name
function r(&$a, $b = 2, &...$rest) {
    $a = 5;
    foreach ($rest as &$x) {
        $x = 9;
    }
}
r($p, b: 3);
var_dump($p);
$y = 0;
r(a: $q, z: $y);
var_dump($q, $y);
t(fn() => r(a: 1));

// variadics: unknown names keep their keys; func_get_args() leaves them out
function v($a, ...$rest) {
    var_dump($rest, func_get_args());
}
v(1, 2, k: 3);

// a class without a constructor takes no named arguments
class NoCtor {}
t(fn() => new NoCtor(x: 1));

// a repeated name, also after an unpack
function d($a, $b = 0) {
}
t(fn() => d(a: 1, a: 2));
t(fn() => d(...[1, 'b' => 2], b: 3));

// native Closure and Generator methods
function gen() {
    $x = yield 1;
    echo "got $x\n";
    yield 2;
}
$g = gen();
$g->current();
var_dump($g->send(value: 'v'));
$c = function ($a, ...$r) {
    var_dump($a, $r);
};
$c->__invoke(a: 1, z: 2);
class Holder {}
$c->call(new Holder, 5, q: 1);
var_dump($c->bindTo(newThis: null, newScope: null) instanceof Closure);
t(fn() => $c->bindTo(foo: 1));
