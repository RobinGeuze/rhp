<?php
interface I {}
class X implements I { function __toString(): string { return 'x'; } }
var_dump(class_implements('X'), (new ReflectionClass('X'))->getInterfaceNames(), new X instanceof Stringable);
abstract class AC implements Countable, IteratorAggregate {}
var_dump(get_class_methods('AC'), array_map(fn($m) => $m->class . '::' . $m->name, (new ReflectionClass('AC'))->getMethods()));
var_dump((new ReflectionClass('Countable'))->getMethods()[0]->name, (string)(new ReflectionMethod('ArrayAccess', 'offsetGet'))->getReturnType());
// attributes with class constants and nested new
class Cfg { const N = 3; }
#[Attribute]
class Opt { function __construct(public $v, public $w = null) {} }
class Use1 {
    #[Opt(Cfg::N * 2, w: [Cfg::class, self::class])]
    public function m(#[SensitiveParameter] $secret) {}
    #[Opt(v: PHP_INT_SIZE)]
    const C = 1;
}
$m = new ReflectionMethod('Use1', 'm');
var_dump($m->getAttributes()[0]->getArguments(), $m->getAttributes()[0]->newInstance()->w);
var_dump($m->getParameters()[0]->getAttributes()[0]->getName());
var_dump((new ReflectionClassConstant('Use1', 'C'))->getAttributes()[0]->getArguments());
// target validation
#[Attribute(Attribute::TARGET_PROPERTY)]
class OnlyProp {}
#[OnlyProp] class Bad {}
try { (new ReflectionClass('Bad'))->getAttributes()[0]->newInstance(); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
#[Opt(1)] #[Opt(2)] class Rep {}
try { (new ReflectionClass('Rep'))->getAttributes()[0]->newInstance(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
class NotAttr {}
#[NotAttr] class Na {}
try { (new ReflectionClass('Na'))->getAttributes()[0]->newInstance(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
#[Missing\Thing(1)] class Mi {}
try { (new ReflectionClass('Mi'))->getAttributes()[0]->newInstance(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump((new ReflectionClass('Mi'))->getAttributes()[0]->getName(), (new ReflectionClass('Mi'))->getAttributes()[0]->getArguments());
// closures & functions
$fn = #[Opt(9)] static fn(int $a = 5): int => $a;
$rf = new ReflectionFunction($fn);
var_dump($rf->getAttributes()[0]->getArguments(), $rf->isStatic(), $rf->getParameters()[0]->getDefaultValue(), $rf->getStartLine());
/** doc for f */
function f(array $a = [], $b = PHP_EOL, ?Cfg $c = null, int|float $d = 1.5) { return 1; }
$rf = new ReflectionFunction('f');
var_dump($rf->getDocComment(), $rf->getStartLine(), $rf->getEndLine(), $rf->getFileName() === __FILE__);
foreach ($rf->getParameters() as $p) { var_dump((string)$p, $p->getDeclaringFunction()->getName(), $p->getDeclaringClass()); }
var_dump(get_class_vars('Opt'), trait_exists('Nope'), interface_exists('Countable'), function_exists('f'));
var_dump(in_array('f', get_defined_functions()['user']), in_array('strlen', get_defined_functions()['internal']));
var_dump(in_array('Cfg', get_declared_classes()), in_array('I', get_declared_interfaces()), get_declared_traits());
var_dump(count(get_included_files()), basename(get_included_files()[0]) === basename(__FILE__));
