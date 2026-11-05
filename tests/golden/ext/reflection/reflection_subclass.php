<?php
final class MyUnion extends ReflectionUnionType {
    public function __construct(private array $types) {}
    public function getTypes(): array { return $this->types; }
    public function allowsNull(): bool { return false; }
    public function __toString(): string { return 'A|B'; }
}
final class MyNamed extends ReflectionNamedType {
    public function __construct(private string $n) {}
    public function getName(): string { return $this->n; }
    public function isBuiltin(): bool { return false; }
    public function allowsNull(): bool { return true; }
    public function __toString(): string { return $this->n; }
}
class MyClass extends ReflectionClass {
    public function __construct(private string $n) {}
    public function getName(): string { return $this->n; }
}
$u = new MyUnion([new MyNamed('A'), new MyNamed('B')]);
var_dump((string) $u, $u instanceof ReflectionType, count($u->getTypes()), (new MyClass('Zz'))->getName());
var_dump((new ReflectionClass('MyUnion'))->getParentClass()->getName(), (new ReflectionClass('ReflectionUnionType'))->isInternal());
function g(): never { throw new Exception(); }
var_dump((string)(new ReflectionFunction('g'))->getReturnType());
$r = new ReflectionObject($u);
var_dump($r->getName(), count($r->getProperties()));
#[AllowDynamicProperties] class D { public $a = 1; }
$d = new D; $d->dyn = 2;
var_dump(array_map(fn($p) => $p->name . ($p->isDynamic() ? '*' : ''), (new ReflectionObject($d))->getProperties()));
var_dump((new ReflectionProperty($d, 'dyn'))->getValue($d));
$arr = [1, 2]; $ref = &$arr[1];
var_dump(ReflectionReference::fromArrayElement($arr, 0), ReflectionReference::fromArrayElement($arr, 1) !== null);
var_dump(Reflection::getModifierNames(ReflectionMethod::IS_ABSTRACT | ReflectionMethod::IS_PROTECTED | ReflectionMethod::IS_STATIC));
$rm = new ReflectionMethod('MyClass', 'getName');
var_dump($rm->getPrototype()->class, $rm->invoke(new MyClass('q')));
$c = $rm->getClosure(new MyClass('w'));
var_dump($c());
