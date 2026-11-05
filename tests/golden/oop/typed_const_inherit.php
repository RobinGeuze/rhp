<?php
// Typed class constants: compatible redeclarations in children, traits
// and implementations link fine.

interface Shape {}
class Circle implements Shape {}

class A {
    const ?Shape S = null;
    const int|string U = 1;
    const iterable IT = [];
    const mixed M = 1;
    public const float F = 2;
}
class B extends A {
    const ?Circle S = null;
    const int U = 2;
    const array IT = [1];
    const int M = 3;
}
trait T {
    const int X = 1;
    const ?array Y = null;
}
class C {
    use T;
    const int X = 1;
    const ?array Y = null;
}
interface HasName { const string NAME = "i"; }
class Named implements HasName { const string NAME = "n"; }

var_dump(B::S, B::U, B::IT, B::M, B::F, C::X, C::Y, Named::NAME);
echo (new ReflectionClassConstant('B', 'U'))->getType(), "\n";
echo (new ReflectionClassConstant('A', 'F'))->getType(), "\n";
