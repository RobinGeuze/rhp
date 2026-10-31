<?php
// Private trait properties are per using class; magic accessors fire only
// for privates the calling scope cannot see; Reflection targets the slot of
// the class the ReflectionProperty was made for.
trait Counter {
    private $count = 0;
    public function inc() { return ++$this->count; }
}
class Base { use Counter; private $tag = 'base'; public function baseTag() { return $this->tag; } }
class Derived extends Base { use Counter; private $tag = 'derived'; public function derivedInc() { return $this->inc(); } }
$d = new Derived;
echo $d->inc(), $d->inc(), ' ', $d->baseTag(), "\n";
var_dump($d);
print_r(get_object_vars($d));
var_dump(property_exists($d, 'count'), property_exists('Base', 'tag'));

class Magic {
    private $hidden = 'h';
    public function __get($n) { return "magic-get($n)"; }
    public function __set($n, $v) { echo "magic-set($n)\n"; }
    public function __isset($n) { echo "magic-isset($n)\n"; return true; }
    public function __unset($n) { echo "magic-unset($n)\n"; }
    public function own() { return $this->hidden; }
}
class MagicChild extends Magic {
    public function childRead() { return $this->hidden; }
    public function childWrite() { $this->hidden = 'child'; }
}
$m = new MagicChild;
echo $m->hidden, "\n";
$m->hidden = 1;
var_dump(isset($m->hidden));
unset($m->hidden);
echo $m->own(), ' ', $m->childRead(), "\n";
$m->childWrite();
echo $m->own(), "\n";
var_dump($m);

class NoMagic { private $p = 1; protected $q = 2; }
$n = new NoMagic;
foreach (['read', 'write', 'unset', 'arraywrite', 'ref'] as $op) {
    try {
        switch ($op) {
            case 'read': echo $n->p; break;
            case 'write': $n->q = 5; break;
            case 'unset': unset($n->p); break;
            case 'arraywrite': $n->p[] = 1; break;
            case 'ref': $r = &$n->q; break;
        }
        echo "$op: no error\n";
    } catch (Error $e) {
        echo "$op: ", $e->getMessage(), "\n";
    }
}
var_dump(isset($n->p), isset($n->q), empty($n->p));

class R1 { private $x = 'r1'; public function x1() { return $this->x; } }
class R2 extends R1 { private $x = 'r2'; public $pub = 1; protected $prot = 2; public function x2() { return $this->x; } }
$r = new R2;
$p1 = new ReflectionProperty(R1::class, 'x');
$p2 = new ReflectionProperty(R2::class, 'x');
echo $p1->getValue($r), ' ', $p2->getValue($r), "\n";
$p1->setValue($r, 'R1-new');
echo $r->x1(), ' ', $r->x2(), "\n";
$p2->setValue($r, 'R2-new');
echo $r->x1(), ' ', $r->x2(), "\n";
foreach ((new ReflectionClass(R2::class))->getProperties() as $prop) echo $prop->getName(), "\n";
foreach ((new ReflectionClass(R1::class))->getProperties() as $prop) echo $prop->getName(), "\n";
var_dump((new ReflectionClass(R2::class))->hasProperty('x'));
