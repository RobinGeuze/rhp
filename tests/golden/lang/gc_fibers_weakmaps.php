<?php
// The cycle collector and WeakMap entries, suspended fibers, the
// destructor fiber and by-reference foreach loops.

class Node { public $other; public $name; function __construct($n) { $this->name = $n; } }
class Dtor {
    public $self;
    function __construct(public string $name) { $this->self = $this; }
    function __destruct() {
        $f = Fiber::getCurrent();
        echo "dtor {$this->name} in ", $f ? get_class($f) . '#' . spl_object_id($f) : 'main', "\n";
    }
}

echo "-- WeakMap value kept while map and key live\n";
$m = new WeakMap;
$k = new Node('k');
$v = new Node('v');
$v->other = $k;
$m[$k] = $v;
unset($v);
var_dump(gc_collect_cycles(), count($m));
unset($k);
var_dump(gc_collect_cycles(), count($m));

echo "-- WeakMap value referring to its map\n";
$m = new WeakMap;
$k = new Node('k');
$m[$k] = [$m, $k];
$w = WeakReference::create($m);
unset($m);
var_dump(gc_collect_cycles(), $w->get() === null);
unset($k);
var_dump(gc_collect_cycles(), $w->get() === null);

echo "-- two maps, one key\n";
$m1 = new WeakMap;
$m2 = new WeakMap;
$k = new Node('k');
$m1[$k] = $m2;
$m2[$k] = $m1;
unset($m1, $m2);
var_dump(gc_collect_cycles());
unset($k);
var_dump(gc_collect_cycles());

echo "-- suspended fiber holding itself\n";
$f = new Fiber(function (Node $n) {
    $self = Fiber::getCurrent();
    $n->other = $self;
    Fiber::suspend($n);
    echo "not reached\n";
});
$n = $f->start(new Node('arg'));
unset($f);
var_dump(gc_collect_cycles());
unset($n);
var_dump(gc_collect_cycles());

echo "-- destructors in the destructor fiber\n";
$outer = new Fiber(function () {
    new Dtor('a');
    new Dtor('b');
    var_dump(gc_collect_cycles());
    new Dtor('c');
    var_dump(gc_collect_cycles());
});
$outer->start();
new Dtor('main');
var_dump(gc_collect_cycles());

echo "-- by-reference foreach holds its array\n";
function loop() {
    $a = [1, 2];
    $r = &$a;
    foreach ($a as &$v) {
        $a[] = &$a;
        $other = 0;
        $a = &$other;
        unset($r);
        var_dump(gc_collect_cycles());
        break;
    }
    var_dump(gc_collect_cycles());
}
loop();

echo "-- a closure key whose value holds it\n";
$m = new WeakMap();
$c = function () {};
$m[$c] = [$c, new stdClass()];
unset($c);
var_dump(gc_collect_cycles(), count($m));
echo "done\n";
