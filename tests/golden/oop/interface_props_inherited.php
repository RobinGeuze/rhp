<?php
// A class lists the properties of its interfaces after its own and its
// parent's (do_interface_implementation); a redeclaration of an inherited
// virtual property gets a slot of its own, in declaration order.
interface I0 { public int $i0 { get; } }
interface I1 extends I0 { public int $i1 { get; } }
interface I2 { public int $i2 { set; } }
trait T { public $t = 1; }
class P implements I2 { public $p = 1; public int $i2; private $pp; }
abstract class C extends P implements I1 { use T; public $c = 1; }
abstract class D extends C implements I2 { public $d; }
foreach (['I1', 'P', 'C', 'D'] as $cl) {
    echo $cl, ':';
    foreach ((new ReflectionClass($cl))->getProperties() as $p) {
        echo ' ', $p->class, '::', $p->name, '(', $p->getModifiers(), ')';
    }
    echo "\n";
    var_dump(array_keys(get_class_vars($cl)));
    var_dump((new ReflectionClass($cl))->hasProperty('i0'));
}
$o = new class extends C { public int $i0 = 0; public int $i1 = 1; };
var_dump($o);
print_r(get_object_vars($o));
print_r((array) $o);
$rc = new ReflectionClass('C');
echo count($rc->getProperties()), "\n";
foreach (['i0', 'i1'] as $n) {
    $rp = new ReflectionProperty('C', $n);
    var_dump($rp->class, $rp->isAbstract(), $rp->isVirtual(), array_keys($rp->getHooks()));
}

// A lazy-proxy shaped subclass (Symfony var-exporter's LazyProxyTrait):
// the abstract properties come from Reflection, interface ones included.
interface HookedInterface { public string $foo { get; } }
abstract class AbstractHooked implements HookedInterface
{
    abstract public string $bar { get; }
    public int $backed {
        get { return $this->backed ??= 234; }
        set { $this->backed = $value; }
    }
}
$code = '';
foreach ((new ReflectionClass('AbstractHooked'))->getProperties() as $p) {
    if ($p->isAbstract()) {
        $code .= "public {$p->getType()} \${$p->name};\n";
    }
}
echo $code;
eval('class Proxy extends AbstractHooked { public $real; public $init; ' . $code . '
    public int $backed {
        get { return $this->real->backed; }
        set(int $value) { $this->real->backed = $value; }
    }
    public function __get($n) {
        echo "__get($n)\n";
        $this->real ??= ($this->init)();
        return $this->real->$n;
    }
}');
$o = (new ReflectionClass('Proxy'))->newInstanceWithoutConstructor();
var_dump(array_keys((array) $o));
$o->init = static fn () => new class extends AbstractHooked { public string $foo = 'Foo'; public string $bar = 'Bar'; };
unset($o->foo, $o->bar);
var_dump($o->foo, $o->bar, $o->backed);

// ce->get_iterator: set by a hooked property declared in the class (or an
// interface) itself, not by one inherited from an interface.
interface I3 extends I0 {}
abstract class E implements I3 {}
foreach (['I0', 'I1', 'I3', 'C', 'E', 'AbstractHooked'] as $cl) {
    echo $cl, ': ', str_contains((string) new ReflectionClass($cl), '<iterateable>') ? 'iterateable' : '-', "\n";
}
