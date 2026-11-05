<?php
class P { public $a = 1; protected $b = 2; private $c = 3; public $n; function q() { return http_build_query($this); } }
class Q extends P { public $d = 4; private $e = 5; protected $f = ['x' => 1]; function q2() { return http_build_query($this); } function q3($o) { return http_build_query(['k' => $o]); } }
class R { function r($o) { return http_build_query($o); } }
$o = new Q;
var_dump($o->q(), $o->q2(), http_build_query($o), (new R)->r($o), $o->q3(new P));
function f($o) { return http_build_query($o, 'p_', '&amp;'); }
var_dump(f($o));
