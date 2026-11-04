<?php
// The declared type a class constant's or property default's TypeError
// prints: self and parent resolve to the class.
define('X', "x");
class B {}
class D extends B {
    const self|int A = X;
    const int|float C = X;
    const ?bool E = X;
    const parent|false F = X;
}
foreach (['A', 'C', 'E', 'F'] as $c) {
    try {
        echo constant("D::$c");
    } catch (TypeError $e) {
        echo $e->getMessage(), "\n";
    }
}
class P extends B {
    public self|int $p = X;
}
try {
    new P;
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
class Q extends B {
    public parent|bool|null $q = X;
}
try {
    new Q;
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
