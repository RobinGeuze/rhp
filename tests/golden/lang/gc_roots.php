<?php
// Where php adds possible roots, seen through gc_status()['roots'], and
// the automatic run's threshold.

function roots() { return gc_status()['roots']; }

// A closure call releases the Closure: it becomes a root (once).
$f = function ($x) { return $x; };
$r0 = roots();
$f(1);
$f(2);
var_dump(roots() - $r0);

// A method call releases `$this`; a static call does not.
class M {
    function m() {}
    static function s() {}
}
$m = new M;
$r0 = roots();
M::s();
var_dump(roots() - $r0);
$m->m();
var_dump(roots() - $r0);

// A built-in class's own calls add none (ReflectionClass::getProperty
// makes its ReflectionProperty without a constructor call).
class C { public $a; }
$rc = new ReflectionClass(C::class);
$rc->getName();
$r0 = roots();
$p = $rc->getProperty('a');
var_dump(roots() - $r0);
$p->getName();
var_dump(roots() - $r0);
unset($p);
var_dump(roots() - $r0);

// A lazy object's initializer is released when it is realized: it is a
// root until freed.
$o = $rc->newLazyGhost(function ($o) {});
$r0 = roots();
$rc->getProperty('a')->setRawValueWithoutLazyInitialization($o, 1);
var_dump(roots() - $r0, $rc->isUninitializedLazyObject($o));
unset($o);

// Native storage is not counted: an SplObjectStorage in a cycle.
function spl_cycle() {
    $a = new stdClass;
    $s = new SplObjectStorage;
    $s[$a] = 1;
    $a->s = $s;
}
spl_cycle();
var_dump(gc_collect_cycles());

// Destructors that make new cycles while the collector runs: the value
// whose release filled the buffer is added after the run without starting
// another, and the threshold grows.
class Cycle {
    public $self;
    function __construct() { $this->self = $this; }
    function __destruct() {
        global $made;
        if ($made++ < 30000) {
            new Cycle();
        }
    }
}
gc_collect_cycles();
$made = 0;
$s0 = gc_status();
for ($i = 0; $i < 10002; $i++) {
    new Cycle();
}
$s = gc_status();
var_dump($s['runs'] - $s0['runs'], $s['threshold'], $made);
// Literal arrays are roots when a variable holding one goes (php without
// opcache); the empty array never is.
function lit() { $a = [1, 2]; $b = []; $c = $b; }
gc_collect_cycles();
$r0 = roots();
lit();
var_dump(roots() - $r0);

// Literal roots stay correct after eval'd code's literals are freed.
for ($i = 0; $i < 50; $i++) {
    eval('$lz = [[1, 2], [3, [4]]];');
}
unset($lz);
gc_collect_cycles();
for ($i = 0; $i < 500; $i++) {
    $cy = [$i];
    $cy[] = &$cy;
    unset($cy);
}
var_dump(gc_collect_cycles());

// A method's first-class callable is a Closure, not an array; the
// SplDoublyLinkedList family keeps native storage.
class FC { public $c; function m() {} }
$fo = new FC;
$fo->c = $fo->m(...);
unset($fo);
var_dump(gc_collect_cycles());
$q = new SplQueue;
$q->enqueue($q);
unset($q);
var_dump(gc_collect_cycles());
echo "done\n";
