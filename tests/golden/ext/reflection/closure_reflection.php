<?php
// getClosureThis of first-class callables (Twig's ReflectionCallable).
class A {
    function getPath($n) {}
    static function s() {}
    function mk() { return [$this->getPath(...), function () {}, static fn() => 1, self::s(...)]; }
    function m() { static $q = 1; $q++; return fn() => $this; }
}
class B extends A {}
$a = new A;
foreach ($a->mk() as $c) {
    $r = new ReflectionFunction($c);
    var_dump($r->name, $r->getClosureThis() === $a, $r->getClosureCalledClass()?->name, $r->getClosureScopeClass()?->name, $r->isStatic());
}
$r = new ReflectionFunction(strlen(...));
var_dump($r->getClosureThis(), $r->getClosureCalledClass());
$fc = Closure::bind($a->getPath(...), new B);
var_dump(get_class((new ReflectionFunction($fc))->getClosureThis()));

// The closure invoke handler: the closure's parameters without defaults
// (ZEND_ACC_USER_ARG_INFO); an internal function's keep theirs.
$c = function ($a, $b = [], $d = PHP_EOL) {};
$m = new ReflectionMethod($c, '__invoke');
var_dump($m->getParameters()[1]->isDefaultValueAvailable());
echo $m, $m->getParameters()[2], "\n";
try {
    $m->getParameters()[1]->getDefaultValue();
} catch (ReflectionException $e) {
    echo $e->getMessage(), "\n";
}
var_dump(get_class($m->getParameters()[0]->getDeclaringFunction()), $m->getParameters()[0]->getDeclaringFunction()->name);
$p = new ReflectionParameter([$c, '__invoke'], 'b');
var_dump($p->getDeclaringFunction()->name, $p->isOptional());
$m = new ReflectionMethod(strpos(...), '__invoke');
var_dump($m->getParameters()[2]->isDefaultValueAvailable());
echo $m;

// ReflectionClass::getMethods lists the invoke handler.
foreach ((new ReflectionClass('Closure'))->getMethods() as $rm) {
    echo $rm->class, '::', $rm->name, ' ', $rm->getNumberOfParameters(), "\n";
}
foreach ((new ReflectionObject($c))->getMethods() as $rm) {
    echo $rm->class, '::', $rm->name, ' ', $rm->getNumberOfParameters(), "\n";
}
foreach ((new ReflectionObject($c))->getMethods(ReflectionMethod::IS_STATIC) as $rm) {
    echo $rm->name, "\n";
}

// getStaticVariables: a closure's bound variables, then its statics.
$x = 1; $y = 2;
$f = function () use (&$x, $y) { static $s; static $t = 3; static $u = PHP_INT_SIZE; static $w = strlen('ab'); };
var_dump((new ReflectionFunction($f))->getStaticVariables());
$f();
var_dump((new ReflectionFunction($f))->getStaticVariables());
var_dump((new ReflectionFunction(fn() => $y + $x))->getStaticVariables());
var_dump((new ReflectionMethod('A', 'm'))->getStaticVariables());
$arrow = $a->m();
var_dump((new ReflectionMethod('A', 'm'))->getStaticVariables());
var_dump((new ReflectionMethod('B', 'm'))->getStaticVariables());
var_dump((new ReflectionFunction($arrow))->getStaticVariables());
var_dump((new ReflectionFunction($a->m(...)))->getStaticVariables());
var_dump((new ReflectionFunction('strlen'))->getStaticVariables());
var_dump((new ReflectionMethod($f, '__invoke'))->getStaticVariables());
function counter() {
    static $n = 0;
    $n++;
    return (new ReflectionFunction(__FUNCTION__))->getStaticVariables();
}
var_dump(counter(), counter());
var_dump((new ReflectionFunction('counter'))->getStaticVariables());
var_dump((new ReflectionFunction(counter(...)))->getStaticVariables());
// Closure's bindTo()/call() through ReflectionMethod, and the invoke
// handler left out of __toString().
$cl = function ($n) { return $this->v + $n; };
$obj = new class { public $v = 40; };
var_dump((new ReflectionMethod('Closure', 'call'))->invoke($cl, $obj, 2));
var_dump((new ReflectionMethod('Closure', 'bindTo'))->invokeArgs($cl, [$obj])(1));
echo new ReflectionObject($cl);
