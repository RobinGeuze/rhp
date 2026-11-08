<?php
class T { public static int $i = 1; public static float $f = 1.0; }
$p = new ReflectionProperty('T', 'i');
$p->setValue(null, '5');
var_dump(T::$i);
$r = &T::$i;
// The property's own type is checked first, then the reference's.
try { $p->setValue(null, 'abc'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { (new ReflectionClass('T'))->setStaticPropertyValue('i', 'abc'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$g = &T::$f;
$p = new ReflectionProperty('T', 'f');
$p->setValue(null, 3);
var_dump(T::$f, $g);
