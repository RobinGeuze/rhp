<?php
// Loose comparison of objects (zend_compare / zend_std_compare_objects).
$o = new stdClass;
var_dump($o == null, null == $o, $o != null, $o < null, null < $o, $o <=> null);
var_dump($o == true, $o == false, $o == "x", "x" == $o, $o == [], [] < $o, $o < []);
var_dump(new stdClass == new stdClass, new stdClass == new ArrayObject);
var_dump(in_array(new stdClass, [new ArrayObject]), in_array(new stdClass, [new ArrayObject, new stdClass]));
$a = new stdClass; $a->x = 1; $b = new stdClass; $b->x = 2;
var_dump($a == $b, $a < $b, $a > $b, $a <=> $b, $b <=> $a);
$b->x = 1; var_dump($a == $b, $a === $b);
$b->y = 0; var_dump($a == $b, $a < $b, $a <=> $b);
$c = new stdClass; $c->y = 1; $d = new stdClass; $d->z = 1;
var_dump($c == $d, $c <=> $d, $d <=> $c);
$n1 = new stdClass; $n1->in = new stdClass; $n1->in->v = [1, 2];
$n2 = new stdClass; $n2->in = new stdClass; $n2->in->v = [1, 2];
var_dump($n1 == $n2); $n2->in->v[] = 3; var_dump($n1 == $n2, $n1 < $n2);
class S { function __construct(public $v) {} function __toString(): string { return "s$this->v"; } }
var_dump(new S(1) == "s1", "s1" == new S(1), new S(1) == "s2", new S(1) < "s2", new S(1) != "s1");
var_dump(new S(1) == new S(1), new S(1) == new S(2), new S(1) < new S(2));
class A { private $p; function __construct($p) { $this->p = $p; } }
class B extends A { private $p = 'b'; }
var_dump(new A(1) == new A(1), new A(1) == new A(2), new A([1]) == new A([1]), new B(1) == new B(1), new B(1) == new B(2), new A(1) == new B(1));
enum E { case X; case Y; }
var_dump(E::X == E::X, E::X == E::Y, E::X == true, E::X == null, E::X != E::Y);
$f = fn() => 1;
var_dump($f == null, null == $f, $f == true);
$r = fopen('php://memory', 'r');
var_dump($r == null, null == $r);
$x = [new stdClass];
var_dump(array_search(new stdClass, $x), array_search(new ArrayObject, $x));
