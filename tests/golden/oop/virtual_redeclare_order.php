<?php
// A redeclaration of an inherited virtual property gets a slot at the end
// (var_dump, (array)), but get_object_vars and foreach over a hooked class
// walk php's property table: the parent's order (zho_build_properties_ex).
class A { public $a = 1; public $v { get => 9; } public $b = 2; }
class B extends A { public $v = 3; public $c = 4; }
var_dump(new B);
print_r((array) new B);
print_r(get_object_vars(new B));
foreach (new B as $k => $x) echo "$k=$x ";
echo "\n";

class L1 { public $x { get => 1; } public $y { get => 2; } public $z = 0; }
class L2 extends L1 { public $x = 10; public $m = 1; }
class L3 extends L2 { public $y = 20; public $n = 2; }
$o = new L3;
var_dump($o);
print_r(get_object_vars($o));
foreach ($o as $k => $x) echo "$k=$x ";
echo "\n", json_encode($o), "\n";

// The child implements the abstract hook with a plain property: it has no
// hooks itself but keeps the parent's hooked iterator.
abstract class F {
    protected $p = 1;
    abstract public $v { get; }
    private $q = 2;
    public $w = 3;
    function it() {
        foreach ($this as $k => $x) echo "$k=$x ";
        echo "\n";
        print_r(get_object_vars($this));
    }
}
class G extends F { public $v = 4; public $g = 5; }
class H extends G {}
$g = new G;
var_dump($g);
$g->it();
foreach ($g as $k => $x) echo "$k=$x ";
echo "\n";
foreach (new H as $k => $x) echo "$k=$x ";
echo "\n";
print_r(get_object_vars($g));
