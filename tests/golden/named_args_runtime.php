<?php
// Named arguments are placed at run time against the callee's parameters:
// methods, static methods, constructors (incl. promoted), closures, arrow
// functions, variadics (extra names are collected), spreads with string
// keys, call_user_func(_array), and non-constant defaults of skipped params.
class V {
    const K = 'k';
    static function s($a = 1, $b = 0) { return [$a, $b]; }
    function m($x, $y = [1], ...$rest) { return [$x, $y, $rest]; }
    function __construct(public $p = 'p', public $q = 'q') {}
    function d($a = PHP_INT_SIZE * 2, $b = self::K, $c = null) { return [$a, $b, $c]; }
}
class W extends V {
    function __construct($z = 'z') { parent::__construct(q: "from $z"); }
}
function show($v) { echo json_encode($v), "\n"; }

show(V::s(b: 1));
show((new V)->m(y: 2, x: 1));
show((new V)->m(1, 2, 3, extra: 4, more: 5));
$v = new V(q: 'Q');
show([$v->p, $v->q]);
$w = new W();
show([$w->p, $w->q]);
show((new V)->d(c: 3));
show((new V)->d(b: 'B', a: 'A'));

function h($a = [1], ...$r) { return [$a, $r]; }
show(h(x: 1));
show(h(...['a' => 5, 'z' => 6]));
show(h(1, ...[2, 3]));

$f = function ($a, $b = 2, $c = 3) { return "$a $b $c"; };
show([$f(1, c: 9), $f(c: 7, a: 0)]);
$g = fn($x, $y = 'y') => "$x$y";
show($g(y: 'Y', x: 'X'));
show(call_user_func_array([new V, 'd'], ['b' => 'B']));
show(call_user_func_array('h', [1, 'k' => 2]));
show(call_user_func('h', a: 5, b: 6));
show(call_user_func_array(fn($x, $y) => $x - $y, ['y' => 1, 'x' => 10]));
$callable = [V::class, 's'];
show($callable(b: 'callable'));

function req($a, $b) { return $a . $b; }
$cases = [
    fn() => req(b: 1),
    fn() => req(1, a: 2),
    fn() => req(1, c: 3),
    fn() => V::s(1, ...['a' => 3]),
    fn() => (new V)->m(y: 1),
    fn() => new V(r: 1),
    fn() => call_user_func_array('h', ['a' => 1, 2]),
];
foreach ($cases as $t) {
    try {
        $t();
    } catch (Error $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}

$calls = 0;
function counter() { global $calls; return ++$calls; }
function nonconst($a = 1, $b = new ArrayObject([1]), $c = 'c', $d = COUNTED) { return [$a, get_class($b), $c, $d]; }
define('COUNTED', 'counted');
show(nonconst(c: 'C'));
show(nonconst(d: 'D', b: new stdClass));
function typed(int $a = 1, string $b = 'x', ?array $c = null) { return [func_num_args(), $a, $b, $c]; }
show([typed(c: [1]), typed(b: 'y')]);
