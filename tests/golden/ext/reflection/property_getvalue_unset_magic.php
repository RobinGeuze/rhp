<?php
// getValue() reads like read_property: an unset() typed slot goes to __get().
class P { public function __get($n) { echo "get $n\n"; return 42; } }
class T extends P { private int $private = 3; public int $pub = 1; private int $never;
    function __construct() { unset($this->private, $this->pub); } }
class C extends T {}
$o = new C;
var_dump((new ReflectionProperty(T::class, 'private'))->getValue($o));
var_dump((new ReflectionProperty(C::class, 'pub'))->getValue($o));
try { (new ReflectionProperty(T::class, 'never'))->getValue($o); } catch (Error $e) { echo $e->getMessage(), "\n"; }
