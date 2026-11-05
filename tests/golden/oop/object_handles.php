<?php
// Object handles are reused once an object is freed (most recently freed
// first), and not during shutdown destructors.
class X { function m() { return spl_object_id($this); } }
function id($o) { return spl_object_id($o); }
var_dump(new X);
var_dump(new X);
echo id(new X), id(new X), "\n";
echo (new X)->m(), (new X)->m(), "\n";
$keep = [new X, new X, new X];
echo id($keep[0]), id($keep[1]), id($keep[2]), "\n";
unset($keep[1]);
echo id(new stdClass), "\n";
$keep = null;
echo id(new stdClass), "\n";
function gen() { yield 1; }
$g = gen();
echo id($g), "\n";
unset($g);
$c = clone new X;
echo id($c), "\n";
$f = fn () => 1;
echo id($f), id(new X), "\n";

class D {
    public function __construct(public $name) {}
    function __destruct() {
        $n = new stdClass;
        echo $this->name, " destructed, new object #", spl_object_id($n), "\n";
    }
}
$d1 = new D('d1');
$d2 = new D('d2');
echo id($d1), id($d2), "\n";
