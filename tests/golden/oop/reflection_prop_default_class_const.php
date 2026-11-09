<?php
// zend_try_ct_eval_class_const: a property default naming a constant the
// class declared earlier is folded at compile time, so Reflection prints
// the value.
enum E { case A; }
class A {
    const X = 1;
    private const P = 'p';
    const ARR = [1, 2];
    public $a = self::X;
    public $b = A::X + 1;
    public $c = [self::P, 'k' => self::ARR, E::A];
    public $d = self::LATER;
    public static $s = self::X * 3;
    const LATER = 5;
    const Y = self::X + 1;
    public $e = self::Y;
}
trait T { const Z = 1; public $t = self::Z; }
class B { use T; }
foreach (['a', 'b', 'c', 'd', 's', 'e'] as $p) echo new ReflectionProperty('A', $p);
echo new ReflectionProperty('B', 't');
