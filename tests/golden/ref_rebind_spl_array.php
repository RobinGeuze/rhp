<?php
// `lval = &$x` rebinds the slot; it never writes through a reference the
// slot already held.
$y = 1;
$a = ['k' => &$y];
$x = 2;
$a['k'] = &$x;
$x = 3;
var_dump($y, $a['k']);

class T
{
    public static $p;
    public static $q;
    public $o;
}
$y = 1;
T::$p = &$y;
$x = 2;
T::$p = &$x;
$x = 5;
var_dump($y, T::$p);
T::$q = &T::$p;
$z = 9;
T::$p = &$z;
var_dump(T::$q, T::$p);
$t = new T();
$y = 1;
$t->o = &$y;
$t->o = &$x;
var_dump($y, $t->o);

// The released value's destructor runs only when nothing holds it.
class D
{
    public static ?D $a = null;
    public static ?D $b = null;
    public function __construct(public int $id)
    {
    }
    public function __destruct()
    {
        echo "dtor {$this->id}\n";
    }
}
D::$a = new D(1);
D::$b = &D::$a;
$tmp = new D(2);
D::$a = &$tmp;
echo "rebound\n";
D::$b = null;
echo "released\n";

// ArrayObject hands out its storage element for nested writes.
$ao = new ArrayObject(['a' => ['x' => 1]]);
$ao['a']['y'] = 2;
$ao['n']['m'] = 3;
$ao['a']['x'] += 10;
unset($ao['a']['y']);
var_dump($ao->getArrayCopy());
$ao['missing']['z'] = 'q';
$ao['missing']['z'] .= '!';
var_dump($ao['missing']);

// foreach by reference over an ArrayObject modifies its storage.
$ao = new ArrayObject([1, 2, 3]);
foreach ($ao as $k => &$v) {
    $v *= 10;
}
unset($v);
var_dump($ao->getArrayCopy());

// A by-reference generator yields references to elements and properties.
function &elems(array &$arr)
{
    foreach (array_keys($arr) as $k) {
        yield $k => $arr[$k];
    }
}
$data = ['a' => 1, 'b' => 2];
foreach (elems($data) as $k => &$e) {
    $e = "$k!";
}
unset($e);
var_dump($data);
