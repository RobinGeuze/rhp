<?php
class D {
    function __construct(public $n) {}
    function __destruct() { echo "dtor {$this->n}\n"; }
}
// A statement: the removed elements are destroyed during the call.
$a = [new D(1), new D(2), new D(3)];
array_splice($a, 0, 2);
echo "after statement\n";
// A used result keeps them until the result goes away.
$a = [new D(4), new D(5), new D(6)];
$r = array_splice($a, 1, 1);
echo "after used\n";
unset($r);
echo "end\n";
$s = ['x' => 1, 2, 3, 'y' => 4];
var_dump(array_splice($s, 1, -1, 'z'), $s);
$s = [1, 2, 3];
var_dump(array_splice($s, 1, PHP_INT_MAX), $s);
