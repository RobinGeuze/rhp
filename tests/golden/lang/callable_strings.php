<?php
// "Class::method" string callables and zend_is_callable_error's messages for
// is_callable / call_user_func / call_user_func_array / dynamic calls.
class K {
    static function st($a = 0) { echo "st $a\n"; return $a * 2; }
    function inst() { echo "inst ", static::class, "\n"; }
    private static function priv() { echo "priv\n"; }
    function test() {
        var_dump(is_callable('K::inst'), is_callable(['K', 'inst']), is_callable('K::priv'));
        call_user_func('K::inst');
        call_user_func('K::priv');
        $f = 'K::priv';
        $f();
    }
}
class Sub extends K {}
class M {
    function __call($n, $a) { echo "__call $n ", json_encode($a), "\n"; }
    static function __callStatic($n, $a) { echo "__callStatic $n ", json_encode($a), "\n"; }
}
enum Suit: string { case H = 'h'; case S = 's'; }
$cases = ['K::st', 'k::ST', '\K::st', 'Sub::st', 'K::nope', 'Nope::x', 'K::inst', ['K', 'inst'],
    ['K', 'nope'], [new K, 'nope'], [1, 2], [new K, 5], ['K'], 'nofn', 'strlen', '\strlen', new K, 3,
    null, 'M::anything', ['M', 'x'], [new M, 'y'], 'K::priv', ['K', 'priv'], 'K::', '::st',
    'K::st::x', [new Sub, 'inst'], 'Suit::from'];
foreach ($cases as $c) {
    echo json_encode(is_callable($c)), ' ';
    try {
        $r = call_user_func($c, 'h');
        echo '=> ';
        var_dump($r);
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
(new K)->test();
foreach (['K::st', 'K::nope', 'Nope::x', 'nofn', 'K::inst', 'K::priv', ['K', 'priv'], [1, 2],
          [new K], 5, true, new K, 'K::', '::st', [new K, 5]] as $c) {
    try {
        $c(9);
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
try { call_user_func_array('nofn', []); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
var_dump(call_user_func_array('K::st', ['a' => 4]));
var_dump(array_map('K::st', [1, 2]));
