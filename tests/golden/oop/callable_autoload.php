<?php
// A callable check (zend_is_callable_check_class) autoloads the class a
// "C::m" string or [C, m] array names: params, variadics, return types,
// is_callable(), internal and prelude callable parameters.
namespace {
spl_autoload_register(function ($c) {
    echo "autoload $c\n";
    if ($c === 'A\F') {
        eval('namespace A; class F { public static function m($x = 0) { return 1; } }');
    } elseif (in_array($c, ['V', 'R', 'P', 'M', 'S', 'I'])) {
        eval("class $c { public static function m(\$a = 0, \$b = 0) { return \$a <=> \$b; } }");
    } elseif ($c === 'Boom') {
        throw new Exception("boom $c");
    }
});
var_dump(is_callable('A\G::x'));
var_dump(is_callable(['A\H', 'x']));
var_dump(is_callable('A\F::m'));

function reg(string $t, callable $f) { echo "reg ok\n"; }
reg('string', 'A\F::m');
function v(callable ...$fs) { echo "v ", count($fs), "\n"; }
v('V::m');
function r($x): callable { return $x; }
var_dump(r(['R', 'm']));
var_dump(array_map('M::m', [1]));
function n(callable|int $x) { var_dump($x); }
n('P::m');
// A string type matches first: no autoload.
function s(callable|string $x) { var_dump($x); }
s('Q::m');
$a = new ArrayObject([3, 1, 2]);
$a->uasort('S::m');
echo implode(',', (array) $a), "\n";
$it = new CallbackFilterIterator(new ArrayIterator([1, 2]), ['I', 'm']);
echo count(iterator_to_array($it)), "\n";

function show(Throwable $e) {
    echo get_class($e), ": ", str_replace(__FILE__, basename(__FILE__), $e->getMessage()), "\n";
    if ($p = $e->getPrevious()) {
        echo "  previous ", get_class($p), ": ", $p->getMessage(), "\n";
    }
}
// A throwing autoloader: the argument check rethrows its exception, the
// return check chains it under the TypeError.
try { reg('x', 'Boom::m'); } catch (Throwable $e) { show($e); }
try { r(['Boom', 'm']); } catch (Throwable $e) { show($e); }
try { r('Z::m'); } catch (Throwable $e) { show($e); }
try { v('V::m', 'Y::m'); } catch (Throwable $e) { show($e); }
}
