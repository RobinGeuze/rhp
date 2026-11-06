<?php
// ReflectionMethod::getPrototype(): only interfaces a class declares itself
// set prototypes; interface methods and inherited ones have none.
interface I1 { function f(); }
interface I2 extends I1 { function f(); function g(); }
interface I3 extends I2 {}
interface J1 { function h(); }
interface J2 extends J1 {}
abstract class A implements I2 { function f() {} abstract function __construct(); }
abstract class K implements J2 { function h() {} }
abstract class L implements I3 {}
class B extends A { function f() {} function g() {} function __construct() {} }
class C extends B implements Countable { function count(): int { return 0; } function f() {} }
class S { function __toString(): string { return ""; } }
$cases = [['I2', 'f'], ['I3', 'f'], ['I2', 'g'], ['A', 'f'], ['A', 'g'], ['K', 'h'], ['L', 'f'],
    ['B', 'f'], ['B', 'g'], ['B', '__construct'], ['C', 'f'], ['C', 'count'], ['C', 'g'], ['S', '__toString']];
foreach ($cases as [$c, $m]) {
    try {
        $p = (new ReflectionMethod($c, $m))->getPrototype();
        echo "$c::$m -> {$p->class}::{$p->name}\n";
    } catch (ReflectionException $e) {
        echo $e->getMessage(), "\n";
    }
}
