<?php
// A class gets php's hooked-property iterator (zend_hooked_object_get_iterator)
// only from hooks its own body or a parent declares; one bound from a trait
// iterates the plain property table (outside Windows).
trait T { public int $x = 1 { get => $this->x * 2; } public $v { get => 'virtual'; } }
class C { use T; public $y = 3; }
class D extends C {}
class H { public $h = 1 { get => $this->h * 10; } }
class HT extends H { use T; }
foreach ([new C, new D, new HT] as $o) {
    echo get_class($o), ': ';
    foreach ($o as $k => $v) echo "$k=$v ";
    echo "\n";
}
foreach (['T', 'C', 'D', 'H', 'HT'] as $c) {
    echo $c, ': ', str_contains((string) new ReflectionClass($c), '<iterateable>') ? 'iterateable' : '-', "\n";
}
var_dump(get_object_vars(new C));
