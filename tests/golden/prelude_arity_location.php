<?php
// Arity errors of prelude-implemented builtins carry the caller's location;
// native Generator/Closure methods and the two-form functions check php's
// counts; a literal for a by-ref builtin parameter is a catchable Error.
function f() {
    try {
        new ArrayObject([], 0, 'ArrayIterator', 5);
    } catch (ArgumentCountError $e) {
        echo $e->getMessage(), ' @', basename($e->getFile()), ':', $e->getLine(), "\n";
    }
    try {
        iterator_to_array();
    } catch (ArgumentCountError $e) {
        echo $e->getMessage(), ' @', basename($e->getFile()), ':', $e->getLine(), "\n";
    }
}
f();
$calls = [
    fn() => (function () { yield 1; })()->send(),
    fn() => (function () { yield 1; })()->current(1),
    fn() => (function () {})->bindTo(),
    fn() => (function () {})->call(),
    fn() => mt_rand(1, 2, 3),
    fn() => mt_rand([]),
    fn() => rand(1),
    fn() => strtr('a'),
    fn() => new WeakReference(null),
    fn() => end([1, 2]),
    fn() => sscanf('1 2', '%d %d', $a, 5),
    fn() => preg_match('/a/', 'a', [1]),
];
foreach ($calls as $c) {
    try {
        $c();
        echo "no error\n";
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
