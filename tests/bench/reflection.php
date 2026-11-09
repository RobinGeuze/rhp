<?php
// Framework-style introspection: ReflectionClass, hasMethod, method_exists,
// class_exists and is_a on every pass (containers, serializers, analysers).
interface I {}
class A implements I { public function run() {} protected function hidden() {} }
class B extends A { public function more() {} }
$names = ['A', 'B', 'I'];
$t = 0;
for ($i = 0; $i < 100000; $i++) {
    $n = $names[$i % 3];
    $r = new ReflectionClass($n);
    $t += $r->hasMethod('run') ? 1 : 0;
    $t += $r->isInterface() ? 2 : 0;
    $t += class_exists($n) ? 4 : 0;
    $t += is_a($n, 'I', true) ? 8 : 0;
    $t += method_exists($n, 'more') ? 16 : 0;
    $t += strlen($r->getShortName());
}
echo $t, "\n";
