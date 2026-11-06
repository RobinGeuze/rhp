<?php
// clone() as a function: named arguments, callables, clone-with in the
// caller's scope, readonly and visibility rules, errors.
class P {
    public $a = 1;
    protected $b = 2;
    private $c = 3;
    public readonly int $r;
    public function __construct() { $this->r = 10; }
    public function with(array $w) { return clone($this, $w); }
    public function __clone() { echo "__clone\n"; }
}
$p = new P;
var_dump(clone(object: $p, withProperties: ['a' => 'A']));
var_dump($p->with(['b' => 'B', 'c' => 'C', 'r' => 11]));
try { clone($p, ['r' => 12]); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { clone($p, ['b' => 'x']); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(array_map('clone', [new ArrayObject([1])])[0]->count());
var_dump(call_user_func('clone', $p, ['a' => 'cuf'])->a);
$f = clone(...);
var_dump($f($p)->a);
var_dump((clone($p, []))->a);

// integer keys become property names
var_dump(clone(new stdClass, [5 => 'five', 'x' => 'ex']));

// errors
foreach ([null, 1, "s", [1]] as $v) {
    try { clone $v; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
}
try { clone($p, "x"); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { clone(); } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n"; }
class Priv { private function __clone() {} public static function make() { return clone new self; } }
try { clone new Priv; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { clone(new Priv, ['x' => 1]); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(Priv::make() instanceof Priv);

// a closure clone-with only creates (forbidden) dynamic properties
try { clone(function () {}, ['p' => 1]); } catch (Error $e) { echo $e->getMessage(), "\n"; }

// an explicit public(set) on a readonly property: anyone may initialize it
final class RO { public public(set) readonly int $x; public readonly int $y; }
$ro = new RO;
$ro->x = 1;
try { $ro->y = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump($ro);
