<?php
// Typed constant redeclarations that name the class being linked (self,
// its own name, static in a final class) are compatible; identical or
// builtin-only checks need no class loading, so the class is early-bound.

var_dump(class_exists('B', false), class_exists('D', false), class_exists('F', false));

interface I {}
class A { const ?A X = null; const ?I Y = null; const ?Foo Z = null; }
class B extends A implements I { const ?self X = null; const ?B Y = null; const ?Foo Z = null; }
final class C extends A { const ?static X = null; }
class D extends A { const ?Foo Z = null; }
class E { const Foo|int N = 1; }
class F extends E { const int N = 2; }

var_dump(class_exists('C', false), B::X, C::X, F::N);
