<?php
// The cycle collector: counts, destructor order, handle reuse, roots.

class N {
    public $other;
    public $name;
    function __construct($name) { $this->name = $name; }
    function __destruct() { echo "destruct {$this->name}\n"; }
}

// A ring of three with destructors: destructed in buffer order, freed by
// the rerun.
$a = new N('a');
$b = new N('b');
$c = new N('c');
$a->other = $b;
$b->other = $c;
$c->other = $a;
unset($a, $b, $c);
var_dump(gc_status()['roots']);
var_dump(gc_collect_cycles());
var_dump(gc_status()['roots']);

// Handles of collected objects are reused, most recently freed first.
$x = new stdClass;
$x->self = $x;
$y = new stdClass;
$y->self = $y;
$ids = [spl_object_id($x), spl_object_id($y)];
unset($x, $y);
var_dump(gc_collect_cycles());
$p = new stdClass;
$q = new stdClass;
var_dump([spl_object_id($p), spl_object_id($q)] == array_reverse($ids));

// Roots from a property, an element and a frame.
$o = new stdClass;
$o->o = $o;
$h = new stdClass;
$h->x = $o;
unset($o);
$h->x = null;
var_dump(gc_status()['roots']);
$arr = [];
$arr[0] = new stdClass;
$arr[0]->a = $arr[0];
unset($arr[0]);
var_dump(gc_status()['roots']);
function f() {
    $l = new stdClass;
    $l->l = $l;
}
f();
var_dump(gc_status()['roots']);
var_dump(gc_collect_cycles());

// A closure that holds itself through a reference.
$fn = function () use (&$fn) { return 1; };
unset($fn);
var_dump(gc_collect_cycles());

// A suspended generator that holds itself.
function g() { $self = yield 1; yield 2; }
$gen = g();
$gen->current();
$gen->send($gen);
unset($gen);
var_dump(gc_collect_cycles());

// Destructor exceptions are chained and thrown from gc_collect_cycles().
class T {
    public $t;
    public $n;
    function __construct($n) { $this->n = $n; }
    function __destruct() { throw new Exception("from {$this->n}"); }
}
$t1 = new T(1);
$t2 = new T(2);
$t1->t = $t2;
$t2->t = $t1;
unset($t1, $t2);
try {
    gc_collect_cycles();
} catch (Exception $e) {
    echo $e->getMessage(), " <- ", $e->getPrevious()->getMessage(), "\n";
}

// gc_disable(): roots still pile up but nothing runs on its own.
gc_disable();
var_dump(gc_enabled(), ini_get('zend.enable_gc'));
for ($i = 0; $i < 12000; $i++) {
    $z = new stdClass;
    $z->z = $z;
}
unset($z);
$s = gc_status();
var_dump($s['runs'], $s['roots'] > 10000);
gc_enable();
var_dump(gc_enabled(), ini_get('zend.enable_gc'));
var_dump(gc_collect_cycles());
echo "done\n";
