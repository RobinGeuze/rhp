<?php
// A class whose typed constant redeclaration needs a class that is not
// loaded yet is not early-bound: it exists once its declaration runs.

var_dump(class_exists('A', false), class_exists('B', false), class_exists('Z', false));

class S implements Stringable { function __toString(): string { return ""; } }
class Z extends S {}
class A { const ?S C = null; }
class B extends A { const ?Z C = null; }

var_dump(class_exists('B', false), B::C);
