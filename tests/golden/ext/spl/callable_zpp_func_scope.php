<?php ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
// A prelude parameter that is php's Z_PARAM_FUNC: checked in the caller's
// scope (deprecations included) and kept resolved, so a `self::` or private
// callable still works when the iterator runs elsewhere.
class K {
    static function f($c, $k = 0, $i = 0) { return true; }
    private function p($c) { return $c > 1; }
    function g() { return new CallbackFilterIterator(new ArrayIterator([1, 2]), ['self', 'f']); }
    function h() { return new CallbackFilterIterator(new ArrayIterator([1, 2, 3]), [$this, 'p']); }
    function r() { return new RecursiveCallbackFilterIterator(new RecursiveArrayIterator([1, [2]]), 'self::f'); }
    function a() { return iterator_apply(new ArrayIterator([1, 2]), ['self', 'f'], [1]); }
}
foreach ((new K)->g() as $v) var_dump($v);
foreach ((new K)->h() as $v) var_dump($v);
foreach (new RecursiveIteratorIterator((new K)->r()) as $v) var_dump($v);
var_dump((new K)->a());
$bad = [
    fn() => new CallbackFilterIterator(new ArrayIterator([1]), 'nope'),
    fn() => new RecursiveCallbackFilterIterator(new RecursiveArrayIterator([1]), 1),
    fn() => iterator_apply(new ArrayIterator([1]), ['K', 'nope']),
    fn() => (new ArrayObject([2, 1]))->uasort('nope'),
    fn() => (new ArrayIterator([2, 1]))->uksort(1),
];
foreach ($bad as $f) {
    try { $f(); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}
