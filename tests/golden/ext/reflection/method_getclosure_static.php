<?php
// A static method's closure ignores the object: its scope and called
// scope are the method's class.
class A { static function s() { return static::class; } function m() { return static::class; } }
class B extends A {}
echo (new ReflectionMethod('A', 's'))->getClosure(new stdClass)(), "\n";
echo (new ReflectionMethod('B', 's'))->getClosure(new B)(), "\n";
echo (new ReflectionMethod('A', 'm'))->getClosure(new B)(), "\n";
try { (new ReflectionMethod('A', 'm'))->getClosure(new stdClass); } catch (ReflectionException $e) { echo $e->getMessage(), "\n"; }
