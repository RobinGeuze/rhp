<?php
// isset / empty / ?? are BP_VAR_IS fetches: an inaccessible property along the
// chain is "not set" (no error), and magic objects get __isset before __get.
class A {
    private $arr = ['k' => 1];
    protected $o;
    public $pub = ['z' => null];
    function __construct() { $this->o = new stdClass; $this->o->x = 1; }
    function inside() { return [isset($this->arr['k']), $this->o->x ?? 'd']; }
}
$a = new A;
$n = 'arr';
var_dump(isset($a->arr['k']), isset($a->o->x), $a->o->x ?? 'd', isset($a?->arr));
var_dump(empty($a->$n), isset($a->$n['k']), empty($a->arr['k']), empty($a->o));
var_dump(isset($a->pub['z']), isset($a->pub['y']), $a->pub['z'] ?? 'null-coalesced');
var_dump($a->inside());
$null = null;
var_dump(isset($null->x->y), $null?->x ?? 'ns', isset($a->nope->deeper));
try { var_dump($a->arr); } catch (Error $e) { echo $e->getMessage(), "\n"; }

class M {
    private $d = ['a' => ['b' => 1], 'f' => 0];
    function __isset($n) { echo "isset $n\n"; return isset($this->d[$n]); }
    function __get($n) { echo "get $n\n"; return $this->d[$n] ?? null; }
}
$m = new M;
var_dump(isset($m->a['b']));
var_dump($m->q ?? 'dq');
var_dump(isset($m->a['zz']));
var_dump(empty($m->f), empty($m->a), empty($m->none));
var_dump($m->d['a']['b'] ?? 'x');

class G {
    function __get($n) { echo "only get $n\n"; return ['k' => 'v']; }
}
$g = new G;
var_dump(isset($g->p['k']), $g->p['k'] ?? 'no');

class I {
    function __isset($n) { echo "only isset $n\n"; return true; }
}
$i = new I;
var_dump(isset($i->p), isset($i->p['k']), $i->p ?? 'fallback');

class S { private static $s = 1; protected static $t; public static $u = ['k' => 2]; }
var_dump(isset(S::$s), isset(S::$t), empty(S::$s), S::$s ?? 'd');
var_dump(isset(S::$u['k']), isset(S::$nope), S::$nope ?? 'undeclared', S::$u['q'] ?? 'missing');
try { var_dump(S::$s); } catch (Error $e) { echo $e->getMessage(), "\n"; }
