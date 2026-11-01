<?php
// Protected access is checked against the root declaration (the prototype),
// so a sibling class can read a property a cousin redeclared.
abstract class Base {
    protected $prot = 'base';
    protected $np = 'np';
    public $pub = 'pub';
}
class C1 extends Base { protected $prot = 'c1'; public $pub = 'c1pub'; }
class C2 extends Base {
    function peek(Base $o) { return $o->np; }
    function peek2(Base $o) { return $o->prot; }
    function poke(Base $o) { $o->prot = 'poked by C2'; }
    function has(Base $o) { return isset($o->prot); }
}
class D1 extends C1 { protected $prot = 'd1'; }
$c1 = new C1; $c2 = new C2; $d1 = new D1;
var_dump($c2->peek($c1), $c2->peek2($c1), $c2->peek2($d1), $c2->has($d1));
$c2->poke($d1);
var_dump($c2->peek2($d1));

// A property first declared in the subclass is not visible to a sibling.
class Root {}
class L extends Root { protected $own = 'own'; }
class R extends Root { function peek(Root $o) { return $o->own; } function has(Root $o) { return isset($o->own); } }
$r = new R;
var_dump($r->has(new L));
try { var_dump($r->peek(new L)); } catch (Error $e) { echo $e->getMessage(), "\n"; }

// Unrelated classes stay denied.
class U { function peek($o) { return $o->prot; } }
try { var_dump((new U)->peek($c1)); } catch (Error $e) { echo $e->getMessage(), "\n"; }

// A parent private redeclared protected in a child starts a new prototype.
class P0 { private $x = 'p0'; }
class P1 extends P0 { protected $x = 'p1'; }
class P2 extends P0 { function peek($o) { return $o->x; } }
try { var_dump((new P2)->peek(new P1)); } catch (Error $e) { echo $e->getMessage(), "\n"; }
