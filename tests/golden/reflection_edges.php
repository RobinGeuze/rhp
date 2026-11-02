<?php
// Reflection edge cases checked against php: setValue goes through the
// normal property write path, static defaults, interface order, clone /
// serialize refusals, closure __invoke, #[Deprecated], parameter lookup.
set_error_handler(function ($no, $str) { echo "[$no] $str\n"; return true; });
function t($f) {
    try {
        $r = $f();
        if (is_object($r)) { echo get_class($r), "\n"; } else { var_dump($r); }
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}

echo "-- setValue\n";
final class P {
    public int $n = 0;
    public $plain = 1;
    public function __construct(public readonly int $ro = 0) {}
}
$o = (new ReflectionClass('P'))->newInstanceWithoutConstructor();
$n = new ReflectionProperty('P', 'n');
t(function () use ($n, $o) { $n->setValue($o, "12"); return $o->n; });
t(fn() => $n->setValue($o, "abc"));
t(fn() => $n->setValue($o, []));
t(fn() => $n->setValue(5, 1));
t(fn() => $n->setValue($o));
$ro = new ReflectionProperty('P', 'ro');
t(function () use ($ro, $o) { $ro->setValue($o, 7); return $o->ro; });
t(fn() => $ro->setValue($o, 8));
$x = 1;
$o->plain = &$x;
(new ReflectionProperty('P', 'plain'))->setValue($o, 42);
var_dump($x);
class M { public $q; public function __set($k, $v) { echo "__set $k\n"; } }
$m = new M;
unset($m->q);
(new ReflectionProperty('M', 'q'))->setValue($m, 3);

echo "-- statics and defaults\n";
class SP { public static $ps = 'p'; public int $pt; }
class SC extends SP { public static $s = 1; public static ?int $t; public int $u; public $v; }
SC::$s = 2;
SP::$ps = 'changed';
var_dump(get_class_vars('SC'));
var_dump((new ReflectionClass('SC'))->getDefaultProperties());
var_dump((new ReflectionProperty('SC', 's'))->getDefaultValue());
$s = new ReflectionProperty('SC', 's');
$s->setValue(9);
var_dump(SC::$s);
$s->setValue('x', 10);
var_dump(SC::$s);
$s->setValue(null, 11);
var_dump(SC::$s);

echo "-- interface order\n";
interface I1 {}
interface I2 {}
interface J extends I1, I2 {}
class A1 implements J { public function __toString(): string { return ''; } }
class A2 extends A1 {}
class A3 extends A2 {}
class A4 extends A3 implements Countable { public function count(): int { return 0; } }
trait TS { public function __toString(): string { return 't'; } }
class WithTrait { use TS; }
foreach (['A1', 'A2', 'A3', 'A4', 'WithTrait', 'Exception', 'ErrorException', 'TypeError', 'ArgumentCountError', 'ReflectionClass', 'ReflectionObject'] as $c) {
    echo $c, ': ', implode(',', (new ReflectionClass($c))->getInterfaceNames()), "\n";
}
var_dump(new WithTrait instanceof Stringable);
var_dump(array_map(fn($m) => $m->name, (new ReflectionClass('Throwable'))->getMethods()));
var_dump((string)(new ReflectionMethod('Stringable', '__toString'))->getReturnType());
var_dump((new ReflectionMethod('Countable', 'count'))->getReturnType());
var_dump((string)(new ReflectionMethod('IteratorAggregate', 'getIterator'))->getTentativeReturnType());

echo "-- clone / serialize\n";
class MyRC extends ReflectionClass {}
class MyEx extends Exception { public function __clone() {} }
t(fn() => serialize(new ReflectionClass('stdClass')));
t(fn() => serialize(new MyRC('stdClass')));
t(fn() => unserialize('O:15:"ReflectionClass":0:{}'));
t(fn() => serialize(WeakReference::create($o)));
t(fn() => clone new ReflectionClass('stdClass'));
t(fn() => clone new Exception('x'));
t(fn() => clone new MyEx('x'));
t(fn() => clone new IteratorIterator(new ArrayIterator([])));
foreach (['MyEx', 'Exception', 'MyRC', 'ArrayObject', 'IteratorIterator', 'Reflection'] as $c) {
    echo $c, ' cloneable: ', var_export((new ReflectionClass($c))->isCloneable(), true), "\n";
}
foreach (['Closure', 'Generator', 'WeakMap', 'Attribute', 'MyRC'] as $c) {
    t(fn() => (new ReflectionClass($c))->newInstanceWithoutConstructor());
}

echo "-- closure __invoke\n";
$cl = function (int $a, &$b, ...$c): int { return $a; };
$im = new ReflectionMethod($cl, '__invoke');
var_dump($im->name, $im->class, $im->isInternal(), $im->getFileName(), $im->getModifiers(), $im->getNumberOfParameters(), (string)$im->getReturnType());
var_dump($im->invoke(fn($a) => $a * 2, 21));
t(fn() => $im->invoke(new stdClass, 1));
var_dump((new ReflectionObject($cl))->hasMethod('__INVOKE'), (new ReflectionObject($cl))->getMethod('__INVOKE')->name, (new ReflectionClass($cl))->hasMethod('__invoke'));
t(fn() => new ReflectionMethod('Closure', '__invoke'));

echo "-- ReflectionParameter lookup\n";
t(fn() => (new ReflectionParameter([$cl, '__invoke'], 1))->name);
t(fn() => (new ReflectionParameter($cl, 'c'))->getPosition());
t(fn() => new ReflectionParameter(42, 0));
t(fn() => new ReflectionParameter(null, 0));
t(fn() => new ReflectionParameter(true, 0));
t(fn() => new ReflectionParameter([1, 2], 0));
t(fn() => new ReflectionParameter(['stdClass'], 0));
t(fn() => new ReflectionParameter(['stdClass', 'nope'], 0));
t(fn() => new ReflectionParameter(new stdClass, 0));
t(fn() => new ReflectionParameter('nope', 0));
t(fn() => new ReflectionParameter('strlen', 3));
t(fn() => new ReflectionParameter('strlen', 'zz'));

echo "-- Deprecated\n";
#[Deprecated] function dd() {}
class K { #[Deprecated] const A = 1; #[Deprecated("x")] public function m() {} const B = 2; }
var_dump((new ReflectionFunction('dd'))->isDeprecated(), (new ReflectionClassConstant('K', 'A'))->isDeprecated(), (new ReflectionClassConstant('K', 'B'))->isDeprecated(), (new ReflectionMethod('K', 'm'))->isDeprecated(), (new ReflectionFunction('strlen'))->isDeprecated());

echo "-- getValue / dynamic properties\n";
class GV { public $u = 1; private $priv = 'p'; public function __get($n) { echo "__get $n\n"; return 'magic'; } }
$gv = new GV;
unset($gv->u);
var_dump((new ReflectionProperty('GV', 'u'))->getValue($gv), (new ReflectionProperty('GV', 'priv'))->getValue($gv));
$dyn = new stdClass;
$dyn->a = 1;
var_dump((new ReflectionClass($dyn))->hasProperty('a'), count((new ReflectionClass($dyn))->getProperties()));
var_dump((new ReflectionObject($dyn))->hasProperty('a'), count((new ReflectionObject($dyn))->getProperties()));
t(fn() => (new ReflectionProperty($dyn, 'a'))->isDynamic());

echo "-- introspection builtins\n";
t(fn() => get_class_vars('NoSuchClass'));
t(fn() => get_class_vars(5));
var_dump(get_class_vars('I1'));
var_dump(count(get_defined_functions(true)) === 2);
