<?php
// ZEND_RETURN frees a call's unused result before the callee's variables
// and $this, so the next object takes php's handle.
class A { function mk() { return new stdClass; } function loc() { $l = new stdClass; return new stdClass; } }
function g() { $l = new stdClass; return new stdClass; }
function same() { $l = new stdClass; return $l; }
$keep = new stdClass;
(new A)->mk(); echo spl_object_id(new stdClass), spl_object_id($a = new stdClass), "\n"; unset($a);
g(); echo spl_object_id(new stdClass), spl_object_id($a = new stdClass), "\n"; unset($a);
same(); echo spl_object_id(new stdClass), spl_object_id($a = new stdClass), "\n"; unset($a);
(new A)->loc(); echo spl_object_id(new stdClass), spl_object_id($a = new stdClass), spl_object_id($b = new stdClass), "\n"; unset($a, $b);
$f = function () { return [new stdClass, new stdClass]; }; $f(); echo spl_object_id(new stdClass), spl_object_id($a = new stdClass), "\n"; unset($a);
$x = g(); echo spl_object_id($x), spl_object_id(new stdClass), "\n";
for ($i = 0, g(); $i < 1; $i++, g()) { echo spl_object_id(new stdClass); } echo "\n";
