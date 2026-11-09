<?php
error_reporting(E_ALL & ~E_DEPRECATED);
// A clone of an ArrayObject gets its own storage, also when another
// ArrayObject shares it or it shares another's.
$a = new ArrayObject([1, 2]);
$b = new ArrayObject($a);
$c = clone $b;
$c[] = 'c';
var_dump(count($a), count($b), count($c));
var_dump($c);
$a2 = clone $a;
$a2[] = 'z';
var_dump(count($a), count($b), count($a2));
class S extends ArrayObject {}
$s = new S([1]);
$t = new ArrayIterator($s);
$u = clone $s;
$u[] = 2;
var_dump(count($s), count($t), count($u));
// An ArrayIterator over another one keeps sharing it.
$i = new ArrayIterator(new ArrayObject([1]));
$j = clone $i;
$j[] = 2;
var_dump(count($i), count($j));
