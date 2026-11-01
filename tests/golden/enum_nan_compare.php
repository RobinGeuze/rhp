<?php
enum E { case X; case Y; }
enum S: string { case A = 'a'; case B = 'b'; }
var_dump(E::X < E::Y, E::X > E::Y, E::X <=> E::Y, E::Y <=> E::X, E::X == E::X, E::X <=> E::X, E::X == E::Y);
var_dump(S::A < S::B, S::B < S::A, S::A <=> S::B, S::A == 'a', 'a' == S::A, S::A <=> 'a', E::X == true, E::X == null, E::X == 1);
var_dump(count(array_unique([E::X, E::Y, E::X], SORT_REGULAR)), array_unique([1, '1', 2, 2.0, 'a'], SORT_REGULAR), array_unique(['10', '1e1', 10.0, 'x'], SORT_NUMERIC));
class D { public $a = 1; }
$f1 = new D; $f2 = new D; unset($f2->a);
var_dump($f2 <=> $f1, $f1 <=> $f2, $f1 == $f2);
class A2 { public $x = 1; } class B2 { public $x = 0; }
var_dump(get_class(min(new A2, new B2)), get_class(max(new A2, new B2)), get_class(min([new A2, new B2])), get_class(max(new A2, new B2, new A2)));
var_dump(NAN >= NAN, NAN <=> 1, 1 <=> NAN, NAN == NAN, NAN != NAN, NAN < 1, NAN > 1, 1 < NAN, 1 > NAN, NAN <= NAN);
var_dump(max(NAN, 1), max(1, NAN), min(NAN, 1), min(1, NAN), max([NAN, 1]), max([1, NAN]), max(1, NAN, 2), min(NAN, 0, 1));
var_dump(null < NAN, NAN == true, NAN == false, NAN == null, NAN <=> "1", "1" <=> NAN, NAN == "NAN", "abc" <=> NAN);
var_dump(null < -5, null == 0.0, null <=> -1, null == [], null < [0], null == "0", "" == null);
var_dump(max(1, 2.0), max(2.0, 1), min(1, 1.0), max('apple', 'banana'), max([1, 2], [1, 3]), max('10', 9), min(PHP_INT_MAX, 9.2e18));
try { max(1); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { min([]); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
$o1 = new stdClass; $o1->a = 1; $o2 = new stdClass; $o2->a = 1; $o2->b = 2;
var_dump($o1 <=> $o2, $o2 <=> $o1);
$o1->c = 2; var_dump($o1 <=> $o2, $o2 <=> $o1, $o1 == $o2);
