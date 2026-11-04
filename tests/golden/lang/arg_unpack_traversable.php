<?php
// `...$x` over Traversables (ZEND_SEND_UNPACK / ZEND_ADD_ARRAY_UNPACK):
// iterators, generators and IteratorAggregate, string keys as named
// arguments, and the unpacking errors. Also `C::m()` on a non-static method
// without a compatible $this.
function t($f) {
    try {
        $f();
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
class A { function m($a = 'A', $b = 'B', ...$r) { echo "m($a,$b) ", json_encode($r), "\n"; } }
function f($a = 'A', $b = 'B', ...$r) { echo "f($a,$b) ", json_encode($r), "\n"; }
$o = new A;
$m = 'm';
$o->m(...new ArrayIterator([10, 20]));
f(...new ArrayIterator([10, 20, 30]));
$o->$m(...new ArrayIterator([10, 20]));
$o->m(...(function () { yield 1; yield 2; })());
f(...(function () { yield 'b' => 1; yield 'a' => 2; })());
f(...(function () { yield '1' => 'x'; yield 5 => 'y'; })());
t(fn() => f(...(function () { yield 1.5 => 1; })()));
t(fn() => f(...(function () { yield 'a' => 1; yield 2; })()));
t(fn() => f(...['a' => 1], ...['a' => 2]));
t(fn() => f(...5));
t(fn() => f(...null));
t(fn() => f(...new stdClass));
t(fn() => [...new stdClass]);
t(fn() => [...(function () { yield null => 1; })()]);
var_dump([...new ArrayIterator(['a' => 1, 2, '7' => 3]), ...(function () { yield 'x' => 9; yield 3 => 4; })()]);
class Agg implements IteratorAggregate { function getIterator(): Iterator { return new ArrayIterator([7, 8]); } }
var_dump([...new Agg]);
f(...new Agg);
class Agg2 implements IteratorAggregate { function getIterator(): Traversable { return new Agg; } }
f(...new Agg2);

class C2 {
    function inst() { echo "inst ", get_class($this), "\n"; }
    function callit() { $n = 'inst'; static::$n(...[]); self::$n(); self::inst(); }
}
class C3 extends C2 { function viaParent() { parent::inst(); C2::inst(); } }
(new C2)->callit();
(new C3)->viaParent();
$n = 'inst';
t(fn() => C2::inst());
t(fn() => C2::inst(...[]));
t(fn() => C2::$n());
t(fn() => C2::$n(...[]));

// Closure methods called by a dynamic name with spread / named arguments.
$cl = function ($a, $b = 0) { echo "closure($a,$b) ", isset($this) ? get_class($this) : '-', "\n"; };
$inv = '__invoke';
$cl->$inv(...[1, 2]);
$cl->$inv(b: 3, a: 4);
$call = 'call';
$cl->$call($o, ...[7]);
$cl->$call($o, b: 9, a: 8);
$bind = 'bindTo';
$b2 = $cl->$bind(...[$o]);
$b2(1);
$num = 5;
t(fn() => $o->$num(...[1]));
t(fn() => $o->$num(1));
