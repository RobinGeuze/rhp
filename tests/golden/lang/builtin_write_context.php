<?php
// Only a call php compiles to a temporary (zend_try_compile_special_func,
// each builtin with its own shape) is "Cannot use result of built-in
// function in write context" at compile time; other calls of the same
// builtins compile, and fail (or not) when they run.
function f() { return [1]; }
function g() {
    // Not array_slice(func_get_args(), N >= 0): an ordinary call.
    array_slice(func_get_args(), -1)[0] = 1;
    return 'g';
}
$a = [1, 2];
$cases = [
    fn() => array_slice($a, 0)[0] = 1,
    fn() => call_user_func('f')[0] = 1,
    fn() => call_user_func_array('f', [])[0] = 1,
    fn() => chr(65)[0] = 1,
    fn() => ord('a')[0] = 1,
    fn() => sprintf('%x', 1)[0] = 1,
    fn() => sprintf('%s', 1, 2)[0] = 1,
    fn() => in_array(1, $a)[0] = 1,
    fn() => in_array(1, [1, 2])[0] = 1,
    fn() => strlen('a', 'b')[0] = 1,
    fn() => g(1, 2),
];
foreach ($cases as $i => $c) {
    try {
        var_dump($c());
    } catch (Throwable $e) {
        echo $i, ': ', get_class($e), ': ', $e->getMessage(), "\n";
    }
}
