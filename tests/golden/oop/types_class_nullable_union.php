<?php
// Class, interface, nullable and union parameter types in strict mode.
declare(strict_types=1);
class A {} class B extends A {} interface I {} class C implements I {}
function needA(A $a): string { return get_class($a); }
function needN(?int $x): string { return $x === null ? "null" : (string)$x; }
function needU(int|string $x): string { return gettype($x); }
function needI(I $x): string { return "i-ok"; }
echo needA(new B), "\n";
echo needN(null), "\n";
echo needU(5), " ", needU("hi"), "\n";
echo needI(new C), "\n";
try { needA(new C); } catch (\TypeError $e) { echo "TE-class\n"; }
try { needN("x"); } catch (\TypeError $e) { echo "TE-null\n"; }
try { needU([1]); } catch (\TypeError $e) { echo "TE-union\n"; }
