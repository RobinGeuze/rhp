<?php
class Sub extends ArrayObject { public $pub = 1; private $priv = 2; }
$a = new ArrayObject(['a'=>1, 'b'=>[2]]);
$i = new ArrayIterator([3, 'x'=>4]);
$s = new Sub(['k'=>'v']);
$e = new ArrayObject();
var_dump(count((array)$e), (array)$a, (array)$s);
echo serialize($a), "\n", serialize($i), "\n", serialize($s), "\n", serialize($e), "\n";
echo json_encode($a), json_encode($i), json_encode($s), json_encode($e), "\n";
var_export($a); echo "\n"; var_export($s); echo "\n";
var_dump($a, $s, $i);
print_r($s); echo "\n";
var_dump(get_object_vars($s), get_object_vars($a));
$u = unserialize(serialize($s)); var_dump($u, $u['k'], count($u));
$u = unserialize(serialize($i)); var_dump($u);
foreach ($s as $k => $v) echo "$k=$v\n";
$f = new ArrayObject(['z'=>1], ArrayObject::STD_PROP_LIST);
var_dump((array)$f, $f->getFlags());
var_dump($a == new ArrayObject(['a'=>1, 'b'=>[2]]), $a == $e);
var_export(new ArrayIterator([3, 'x' => 4])); echo "\n";
// composer JsonManipulator::format: an empty ArrayObject casts to [].
var_dump(count((array) new \ArrayObject()));
// numeric property names stay string keys in foreach and serialize.
$o = json_decode('{"0":"a","12":"b"}');
foreach ($o as $k => $v) var_dump($k);
echo serialize($o), "\n";
$p = new stdClass; $p->{'7'} = 1; echo serialize($p), "\n";
// a user subclass's own "__" property is a real property.
class E2 extends Exception { public $__meta; }
$a = new E2('x'); $b = new E2('x'); $b->__meta = 1;
var_dump($a == $b, array_key_exists('__meta', get_object_vars($a)));
var_dump(get_mangled_object_vars(new Sub([1])));
