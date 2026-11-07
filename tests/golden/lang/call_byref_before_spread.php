<?php
// A variable sent before a spread / named argument to a callee known only at
// run time: SEND_VAR_EX asks the callee whether the parameter is by reference.
class C { function el(&$o, $d, $x = 0, $y = 0) { $o .= "[$d]"; } static function sel(&$o, $d, $x = 0) { $o .= "[$d]"; } function v($o, $x, ...$r) { $o .= "!"; return $o; } }
function id($a) { return $a; }
function g() { return 'G'; }
function uf(&$o, $x, ...$r) { $o .= "{uf$x}"; }
$c = new C;
$a = ''; $c->el($a, 'm-lit1', ...[1]); echo $a, "|";
$a = ''; $c->el($a, 'm-lit0', ...[]); echo $a, "|";
$a = ''; $c->el($a, 'm-call1', ...id([1])); echo $a, "|";
$e = []; $a = ''; $c->el($a, 'm-cv0', ...$e); echo $a, "|";
$e = [1]; $a = ''; $c->el($a, 'm-cv1', ...$e); echo $a, "|";
$a = ''; C::sel($a, 's-lit0', ...[]); echo $a, "|";
$a = ''; $c->el($a, 'm-plain'); echo $a, "|";
$a = ''; $c->el($a, ...['m-only']); echo $a, "|";
$a = ''; $c->el($a, g(), y: 1); echo $a, "\n";
$s = 'v'; var_dump($c->v($s, g(), ...[])); var_dump($s);
$a = ['k' => '']; $c->el($a['k'], g(), ...[]); var_dump($a);
$o = new stdClass; $o->p = ''; $c->el($o->p, 'z', ...[]); var_dump($o);
$s = ''; $m = 'el'; $c->$m($s, g(), ...[]); var_dump($s);
$s = ''; $f = [$c, 'el']; $f($s, g(), ...[]); var_dump($s);
$s = ''; $f = 'uf'; $f($s, g(), ...[]); var_dump($s);
$s = ''; uf2($s, g(), ...[]); var_dump($s);
$s = ''; new D($s, g(), ...[]); var_dump($s);
// a later argument changes the variable: a by-value parameter gets the old value
$s = 'old'; function chg(&$s) { $s = 'new'; return 1; }
var_dump($c->v($s, chg($s), ...[]));
function uf2(&$o, $x, ...$r) { $o .= "{uf2$x}"; }
class D { function __construct(&$o, $x) { $o = "D$x"; } }
// Walker::display_element's call shape
class Walker { function start_el(&$output, $data_object, $depth = 0, $args = [], $current_object_id = 0) { $output .= "<li>$data_object</li>"; }
  function display($el, $args) { $output = ''; $this->start_el($output, $el, 0, ...array_values($args)); return $output; } }
var_dump((new Walker)->display('item', ['a' => [1], 'b' => 7]));
