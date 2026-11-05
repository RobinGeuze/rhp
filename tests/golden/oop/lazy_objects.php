<?php
// Lazy objects: triggers and non-triggers the phpt corpus does not pin.
class P { public $a = 1; protected $b = 2; private $c = 3; public $d; }
$r = new ReflectionClass(P::class);
$mk = fn() => $r->newLazyProxy(function () { echo "init\n"; return new P; });
$mg = fn() => $r->newLazyGhost(function () { echo "init\n"; });

echo "== get_object_vars\n";
var_dump(get_object_vars($mk()));
echo "== json_encode\n";
echo json_encode($mg()), "\n";
echo "== compare\n";
var_dump($mk() == new P, $mg() == $mg());
echo "== foreach by reference\n";
$o = $mk();
foreach ($o as $k => &$v) { $v = "x$k"; }
unset($v);
var_dump($o->a);
echo "== (array) and get_mangled_object_vars do not initialize\n";
$o = $mg();
var_dump(count((array) $o), get_mangled_object_vars($o), $r->isUninitializedLazyObject($o));
echo "== array_splice converts the object\n";
$a = [];
array_splice($a, 0, 0, new P);
var_dump(count($a));
echo "== isLazy\n";
$o = $mg();
$rp = new ReflectionProperty(P::class, 'a');
var_dump($rp->isLazy($o));
$rp->skipLazyInitialization($o);
var_dump($rp->isLazy($o), $r->isUninitializedLazyObject($o));
echo "== dynamic property name from __toString\n";
$n = new class { function __toString(): string { return 'a'; } };
$p = new P;
$p->$n = 5;
var_dump($p->a);
echo "== reference to an uninitialized non-nullable property\n";
class T { public int $i; public ?int $j; }
$t = new T;
try { $x = &$t->i; } catch (Error $e) { echo $e->getMessage(), "\n"; }
$y = &$t->j;
var_dump($t->j);
echo "== ReflectionProperty::__toString\n";
echo new ReflectionProperty(T::class, "j");
echo new ReflectionProperty(T::class, 'i');
