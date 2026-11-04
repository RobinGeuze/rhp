<?php
// __sleep(): each name is tried as is, then as this class's private, then as
// protected; unknown names are skipped and repeats dropped (php warns, here
// silenced with @).
class P { private $pp = 'pp'; protected $prot = 'prot'; }
class C extends P {
    public $pub = 1;
    private $mine = 'm';
    function __sleep() { return ['pp', 'pub', 'nope', 'pub', 'prot', 'mine']; }
}
echo str_replace("\0", '~', @serialize(new C)), "\n";
class E2 { public $a = 1; public $b = 2; function __sleep() { return ['a', 'a', 'b']; } }
echo @serialize(new E2), "\n";
class D { function __sleep() { return 5; } }
echo @serialize(new D), "\n";
echo @serialize([new D, 1]), "\n";
class N { public $x = 1; public $y; function __sleep() { return ['y', 'x']; } }
echo serialize(new N), "\n";
class U { public $gone = 1; public $kept = 2; function __sleep() { unset($this->gone); return ['gone', 'kept']; } }
echo @serialize(new U), "\n";
class Num { public $a = 'A'; function __sleep() { return [0 => 'a']; } }
echo serialize(new Num), "\n";
class Shared { public $o; public $o2; function __sleep() { return ['o', 'o2']; } }
$s = new Shared; $s->o = new stdClass; $s->o2 = $s->o;
echo serialize($s), "\n";
