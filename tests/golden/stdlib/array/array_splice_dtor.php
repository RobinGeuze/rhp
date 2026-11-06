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
// Any write by a destructor counts as a modification, even one that keeps
// the length; a later destructor still runs from the old array.
class W {
    function __construct(public $n) {}
    function __destruct() {
        global $w;
        echo "W{$this->n}\n";
        if ($this->n == 1) {
            $w[0] = 'over';
        }
    }
}
$w = ['keep', new W(1), new W(2), 'x'];
try {
    array_splice($w, 1, 2);
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
var_dump($w);
$w = null;
echo "done\n";
