<?php
// ReflectionClass, ReflectionMethod and ReflectionProperty on a namespaced
// class with a parent, an interface and a promoted constructor.
namespace App;
interface Greets {}
abstract class Base { const VERSION = '1.0'; public $x = 1; }
class Thing extends Base implements Greets {
    private $secret = 42;
    public function __construct(public int $id = 7) {}
    public function hi(string $n): string { return "hi $n #$this->id"; }
}
$r = new \ReflectionClass(Thing::class);
echo $r->getName(), " / ", $r->getShortName(), " / ", $r->getNamespaceName(), "\n";
var_dump($r->isAbstract(), $r->isInstantiable());
echo $r->getParentClass()->getName(), "\n";
var_dump($r->implementsInterface(Greets::class), $r->isSubclassOf(Base::class));
$o = $r->newInstance(99);
$m = new \ReflectionMethod(Thing::class, 'hi');
echo $m->invoke($o, 'bob'), "\n";
foreach ($m->getParameters() as $p) {
    echo $p->getName(), ":", ($p->getType() ? $p->getType()->getName() : '?'), " ";
}
echo "\n";
$prop = new \ReflectionProperty($o, 'secret');
echo $prop->getValue($o), "\n";
$prop->setValue($o, 100);
echo $prop->getValue($o), "\n";
echo $r->getConstant('VERSION'), "\n";
