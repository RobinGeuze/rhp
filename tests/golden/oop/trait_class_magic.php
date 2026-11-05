<?php
// __CLASS__ inside a trait names the using class everywhere; __TRAIT__ and
// __METHOD__ keep the trait name. Statics in trait methods are per using class.
trait T {
    const TC = __CLASS__;
    public static $s = __CLASS__;
    public $p = __CLASS__;
    public $arr = [__CLASS__ => 1];
    function f($a = __CLASS__) {
        static $z = __CLASS__;
        $c = function () { return __CLASS__; };
        return [$a, $z, __CLASS__, __TRAIT__, __METHOD__, $c(), (fn() => __CLASS__)()];
    }
    static function g() { return __CLASS__; }
    function count() { static $n = 0; return ++$n; }
    static function sc() { static $c = self::class; static $k = 0; return $c . ++$k; }
}
class A { use T; }
class B { use T; }
class C extends A {}
var_dump(A::TC, A::$s, (new A)->p, (new A)->arr, (new A)->f());
var_dump(B::TC, B::$s, (new B)->p, (new B)->f(), B::g(), A::g());
var_dump(C::g(), (new C)->f()[2], C::TC);
echo (new A)->count(), (new A)->count(), (new B)->count(), (new C)->count(), "\n";
echo A::sc(), B::sc(), C::sc(), A::sc(), "\n";
class P { function m() { static $n = 0; return ++$n; } }
class Q extends P {}
echo (new P)->m(), (new Q)->m(), (new P)->m(), "\n";
function fs() { static $n = 0; return ++$n; }
echo fs(), fs(), "\n";
