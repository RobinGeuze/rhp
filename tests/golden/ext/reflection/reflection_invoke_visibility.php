<?php
// ReflectionMethod::invoke()/invokeArgs() ignore visibility (private,
// protected, static), and check the object / static-ness like php.
class P {
    private function hid($x) { return "hid $x"; }
    protected function prot(...$a) { return implode(',', $a); }
    private static function st() { return "st " . static::class; }
}
class C extends P {}
var_dump((new ReflectionMethod('P', 'hid'))->invoke(new P, 1));
var_dump((new ReflectionMethod('P', 'prot'))->invokeArgs(new C, [1, 2, 3]));
var_dump((new ReflectionMethod('P', 'st'))->invoke(null));
try { (new ReflectionMethod('P', 'hid'))->invoke(null, 1); } catch (ReflectionException $e) { echo $e->getMessage(), "\n"; }
try { (new ReflectionMethod('P', 'hid'))->invoke(new stdClass, 1); } catch (ReflectionException $e) { echo $e->getMessage(), "\n"; }
