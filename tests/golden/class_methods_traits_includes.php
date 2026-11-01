<?php
// get_class_methods (scope-filtered, php member order), trait_exists,
// get_included_files / get_required_files, Phar::running outside a phar.
trait T { public function t1() {} private function t2() {} }
abstract class P { public function p1() {} protected function p2() {} private function p3() {} public static function ps() {} }
class C extends P {
    use T;
    public function c1() {}
    private function c2() {}
    protected function p2() {}
    static function inside() { return get_class_methods('C'); }
}
class D { static function f() { return get_class_methods('C'); } }
class E extends C { static function g() { return get_class_methods(new C); } }
var_dump(get_class_methods('C'), get_class_methods(new C), C::inside(), D::f(), E::g(), get_class_methods('T'));
var_dump(trait_exists('T'), trait_exists('C'), trait_exists('Nope'), trait_exists('t', false));
try { get_class_methods('Nope'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }

$f = tempnam(sys_get_temp_dir(), 'gi');
file_put_contents($f, '<?php return 1;');
include $f;
include $f;
require_once $f;
$inc = get_included_files();
var_dump(count($inc), $inc[0] === __FILE__, $inc[1] === realpath($f), get_required_files() === $inc);
unlink($f);
var_dump(Phar::running(), Phar::running(false));
