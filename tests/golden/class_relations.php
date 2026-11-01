<?php
interface A {}
interface B extends A {}
interface C {}
interface D extends B, C {}
interface E {}
trait T1 {}
trait T2 { use T1; }
class Base implements C {}
class Mid extends Base implements D { use T2; }
class Leaf extends Mid implements E, A { use T1, T2; }

print_r(class_implements('Leaf'));
print_r(class_implements(new Mid()));
print_r(class_implements('D'));
print_r(class_implements('Base'));
print_r(class_parents('Leaf'));
print_r(class_parents(new Base()));
print_r(class_uses('Leaf'));
print_r(class_uses('Mid'));
print_r(class_uses('T2'));
print_r(class_uses('Base'));
var_dump(@class_implements('NoSuchClass', false));
var_dump(@class_parents('NoSuchClass', false));
try {
    class_implements(42);
} catch (TypeError $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
echo key(class_implements('Leaf')), "\n";

// autoload flag
spl_autoload_register(function ($c) {
    echo "autoload($c)\n";
});
var_dump(@class_parents('Ghost', false));
var_dump(@class_parents('Ghost'));

// class_alias
var_dump(class_alias('Leaf', 'LeafAlias'));
$o = new LeafAlias();
var_dump(get_class($o), $o instanceof Leaf, $o instanceof Mid);
var_dump(class_exists('leafalias'));
var_dump(@class_alias('Leaf', 'LeafAlias'));
var_dump(@class_alias('Leaf', 'Base'));
var_dump(@class_alias('Missing', 'Whatever', false));
var_dump(class_alias('D', 'DAlias'));
var_dump(new Leaf() instanceof DAlias);
var_dump(interface_exists('DAlias'));
print_r(class_implements('LeafAlias'));
function takes(LeafAlias $x) { return get_class($x); }
var_dump(takes(new Leaf()));
