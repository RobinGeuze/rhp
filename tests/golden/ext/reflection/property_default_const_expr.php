<?php
enum S: string { case U = 'unverified'; }
class P { const C = 3; }
final class D extends P {
    public S $status = S::U;
    public ?S $n = null;
    public static $st = self::C + 1;
    public $arr = [self::C, S::U];
    private $q = PHP_INT_MAX;
    public int $z = PHP_INT_SIZE * 2;
    public $e = E_ALL;
    public $w = [1, 2][0];
    public $t = __CLASS__ . 'x';
}
foreach (['status', 'n', 'st', 'arr', 'q', 'z', 'e', 'w', 't'] as $p) {
    $r = new ReflectionProperty(D::class, $p);
    echo $r;
    var_dump($r->hasDefaultValue(), $r->getDefaultValue());
}
var_dump((new ReflectionClass(D::class))->getDefaultProperties());
new D;
// After the class's constants are updated an instance default shows its
// value; a static's default stays the expression.
echo new ReflectionProperty(D::class, 'status');
echo new ReflectionProperty(D::class, 'st');
class Bad { public $x = UNDEF_CONST; }
try { var_dump((new ReflectionProperty(Bad::class, 'x'))->getDefaultValue()); } catch (Error $e) { echo $e->getMessage(), "\n"; }
