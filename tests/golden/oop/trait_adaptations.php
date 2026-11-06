<?php
// Trait adaptation rules and composition that phpt does not pin.

trait Counter {
    public static function hit() {
        static $n = 0;
        return ++$n;
    }
    protected function secret() { return __FUNCTION__ . '/' . __METHOD__; }
}

trait Greeter {
    public function hello() { return 'Greeter::hello'; }
    public function bye() { return 'Greeter::bye'; }
}

trait Polite {
    public function hello() { return 'Polite::hello'; }
}

class A {
    use Counter, Greeter, Polite {
        Polite::hello insteadof Greeter;
        Greeter::hello as greeterHello;
        hit as hitAgain;
        secret as public reveal;
        bye as protected;
    }
}

$a = new A;
echo $a->hello(), "\n", $a->greeterHello(), "\n", $a->reveal(), "\n";
var_dump(A::hit(), A::hit(), A::hitAgain(), A::hit());
$r = new ReflectionClass('A');
var_dump($r->getTraitAliases());
foreach (['hello', 'greeterHello', 'reveal', 'secret', 'bye', 'hitAgain'] as $m) {
    $rm = $r->getMethod($m);
    echo $m, ': ', $rm->getName(), ' ', implode(' ', Reflection::getModifierNames($rm->getModifiers())),
        ' declared in ', $rm->getDeclaringClass()->getName(), "\n";
}
print_r(get_class_methods('A'));
try { $a->bye(); } catch (Error $e) { echo $e->getMessage(), "\n"; }

// A class method wins over a trait's; a trait's wins over the parent's.
class Base { public function hello() { return 'Base::hello'; } }
class B extends Base { use Greeter; }
class C extends Base { use Greeter; public function hello() { return 'C::hello'; } }
echo (new B)->hello(), ' ', (new C)->hello(), "\n";

// The same trait reached twice is not a collision.
trait Inner { public function inner() { return 'inner'; } }
trait Left { use Inner; }
trait Right { use Inner; }
class D { use Inner, Inner; }
echo (new D)->inner(), "\n";

// Compatible trait constants and properties compose silently.
trait K1 { public const X = 1; public $p = [1, 2]; public static $s = 'a'; }
trait K2 { public const X = 1; public $p = [1, 2]; public static $s = 'a'; }
class E { use K1, K2; public const X = 1; public $p = [1, 2]; }
var_dump(E::X, (new E)->p, E::$s);

// Abstract trait methods satisfied by the parent, and through an alias.
trait Needs { abstract public function need(): string; public function use_() { return $this->need(); } }
class P { public function need(): string { return 'from parent'; } }
class F extends P { use Needs; }
echo (new F)->use_(), "\n";

// Static trait members accessed on the trait itself.
trait St { public static $v = 5; public static function m() { return 'm'; } public static function __callStatic($n, $a) { return "cs:$n"; } }
set_error_handler(function ($no, $str) { echo "E$no: $str\n"; return true; });
var_dump(St::$v, isset(St::$v), St::m(), St::other());
try { echo St::class, "\n"; var_dump(constant('K1::X')); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(defined('K1::X'));
