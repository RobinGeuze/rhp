<?php
trait T { public $t; public static $ts; public function tm() {} }
interface I { function im(); }
abstract class A implements I { public $a; private $pa; public static $s; protected $x; public function am() {} private function apriv() {} }
class B extends A { use T; public $b; protected $x = 2; public static $s2; public function bm() {} public function im() {} }
foreach (['A', 'B'] as $c) {
  $r = new ReflectionClass($c);
  echo $c, ": props ", implode(',', array_map(fn($p) => $p->class . '::' . $p->name, $r->getProperties())), "\n";
  echo $c, ": methods ", implode(',', array_map(fn($p) => $p->class . '::' . $p->name, $r->getMethods())), "\n";
  echo $c, ": get_class_methods ", implode(',', get_class_methods($c)), "\n";
  var_dump(get_class_vars($c));
  var_dump($r->getDefaultProperties(), $r->getStaticProperties(), $r->getModifiers());
}
var_dump(array_slice(get_declared_classes(), -2), array_slice(get_declared_interfaces(), -1), get_declared_traits());
$before = get_declared_classes();
eval('class Late {} trait LateT {}');
var_dump(array_values(array_diff(get_declared_classes(), $before)), trait_exists('LateT'), trait_exists('Late'), class_exists('LateT'));
spl_autoload_register(function ($c) { echo "autoload($c)\n"; if ($c === 'Auto') { eval('class Auto { public $p = 1; }'); } });
var_dump((new ReflectionClass('Auto'))->getName(), trait_exists('NoSuchTrait'), trait_exists('NoSuchTrait2', false), get_class_vars('Auto'));
class Scope { private $priv = 1; protected $prot = 2; public $pub = 3; private function pm() {} protected function qm() {} public function rm() {}
  static function inside() { var_dump(get_class_methods('Scope'), get_class_vars('Scope')); } }
Scope::inside();
var_dump(get_class_methods('Scope'), get_class_vars('Scope'), get_class_methods(new Scope));
try { get_class_methods('Nope'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$f = get_defined_functions();
var_dump(array_keys($f), in_array('strlen', $f['internal']), $f['user']);
function userfn() {}
var_dump(get_defined_functions()['user'], count(get_included_files()));
