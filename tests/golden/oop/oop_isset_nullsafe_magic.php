<?php
// isset()/empty() over a nullsafe property fetch consult __isset (and __get
// only for empty() once __isset agreed), like the plain `->` form.
class M {
    private $d = ['a' => ['b' => 1], 'n' => null, 'z' => 0];
    private $priv = 5;
    function __isset($n) { echo "isset $n\n"; return array_key_exists($n, $this->d); }
    function __get($n) { echo "get $n\n"; return $this->d[$n] ?? null; }
}
$m = new M;
var_dump(isset($m?->a));
var_dump(isset($m?->n));
var_dump(isset($m?->missing));
var_dump(empty($m?->a));
var_dump(empty($m?->z));
var_dump(empty($m?->missing));
var_dump(isset($m?->a['b']));
var_dump($m?->a ?? 'dflt');
$name = 'a';
var_dump(isset($m?->$name), empty($m?->$name));
$z = null;
var_dump(isset($z?->a), empty($z?->a), isset($z?->a->b), empty($z?->a->b), $z?->a ?? 'n');
class P { private $p = 1; protected $q = 2; public $r = null; }
$p = new P;
var_dump(isset($p?->p), isset($p?->q), isset($p?->r), empty($p?->p), empty($p?->q));
