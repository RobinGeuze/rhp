<?php
// An ArrayObject over an object writes offsets through to the object's
// property table; append() is refused.
class T { public $p = 1; }
$t = new T;
$ao = new ArrayObject($t);
$ao['p'] = 5;
$ao['q'] = 6;
$ao["\0A\0b"] = 7;
unset($ao['p']);
var_dump(get_object_vars($t));
var_dump(count($ao), isset($ao['q']), $ao['q']);
try { $ao->append(1); } catch (Error $e) { echo $e->getMessage(), "\n"; }
class X extends ArrayObject {}
$x = new X(new T);
try { $x->append(1); } catch (Error $e) { echo $e->getMessage(), "\n"; }
$x->offsetSet('n', 2);
$x->offsetUnset('p');
var_dump($x->getArrayCopy());
// exchangeArray() moves the write-through to the new object.
$u = new T;
$ao->exchangeArray($u);
$ao['w'] = 1;
var_dump(get_object_vars($t), get_object_vars($u));
$ao->exchangeArray([]);
$ao['v'] = 1;
$ao->append(2);
var_dump(get_object_vars($u), $ao->getArrayCopy());

// Unsetting the last reference to an element destructs it right away.
class D { function __destruct() { echo "D::__destruct\n"; } }
class H { public $d; }
$h = new H; $h->d = new D;
$ao = new ArrayObject($h);
$h = null;
unset($ao['d']);
echo "after unset\n";
