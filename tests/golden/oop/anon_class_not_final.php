<?php
// Anonymous classes are not final: an alias of one can be extended
// (Symfony var-exporter's lazy proxies of anonymous classes).
$obj = new class extends \stdClass {
    public function collect(...$args): array { return $args; }
    public function self(): static { return $this; }
};
$r = new ReflectionClass($obj);
var_dump($r->isFinal(), $r->getModifiers(), $r->isAnonymous());
var_dump((new ReflectionClass(new class {}))->isFinal());
class_alias($r->name, 'AnonAlias');
eval('class AnonChild extends AnonAlias {
    public function collect(...$args): array { return array_reverse(parent::collect(...$args)); }
    public function self(): static { return $this; }
}');
$x = new AnonChild;
var_dump($x->collect(1, 2, 3), get_parent_class($x) === $r->name, $x instanceof AnonAlias);
var_dump((new ReflectionClass($x))->isFinal(), $x->self() === $x);
