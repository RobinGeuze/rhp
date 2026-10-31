<?php
class B {
    private $v;
    protected $i;
    public $p = 1;
    function __construct($v, $i) { $this->v = $v; $this->i = $i; }
}
#[AllowDynamicProperties]
class C { public $p = 1; }
$a = new B('8.0', true);
var_dump($a == $a, $a == new B('8.0', true), $a == new B('0.0', true), $a == new B('8.0', false));
var_dump(new B('1', 1) < new B('2', 1), new B('2', 1) > new B('1', 1), new B('1', 1) <=> new B('1', 2));
var_dump(new C == new C, new C == new B(1, 1), new C != new B(1, 1));
$x = new C; $y = new C; $y->extra = 2;
var_dump($x == $y, $x < $y, $y > $x);
$x->extra = 2;
var_dump($x == $y);
var_dump(new C == null, null == new C, new C > null, null < new C, new C == true, new C == false);
var_dump(in_array(new C, ['x', new C]), in_array(new B(1, 2), [new B(1, 3)]));
var_dump(array_search(new B(5, 5), [new B(1, 1), new B(5, 5)]));
$f = fn() => 1;
var_dump($f == $f, $f == fn() => 1);
var_dump(new stdClass == new stdClass, (object)['a' => 1] == (object)['a' => 1], (object)['a' => 1] == (object)['a' => 2]);
var_dump((object)['a' => 1, 'b' => 2] == (object)['b' => 2, 'a' => 1]);
var_dump(new ArrayObject([]) == new stdClass);
