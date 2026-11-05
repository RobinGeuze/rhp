<?php
// serialize() numbers every value like php's var_hash: a repeated object is
// r:n, a repeated PHP reference R:n, so cycles terminate and identity /
// references survive a round trip.
$o = new stdClass; $o->self = $o;
echo serialize($o), "\n";
$u = unserialize(serialize($o));
var_dump($u->self === $u);

$p = new stdClass; $p->v = 1;
echo serialize([$p, $p, 'x' => [$p]]), "\n";
$u = unserialize(serialize([$p, $p]));
var_dump($u[0] === $u[1]);
$u[0]->v = 2;
var_dump($u[1]->v);

$x = 1;
echo serialize([&$x, &$x, $x]), "\n";
$a = [1]; $a[] = &$a;
echo serialize($a), "\n";
$b = ['x' => [1]]; $b['y'] = &$b['x'];
echo serialize($b), "\n";
$ub = unserialize(serialize($b));
$ub['y'][] = 2;
echo json_encode($ub), "\n";
$r = unserialize('a:2:{i:0;i:1;i:1;R:2;}');
$r[0] = 5;
print_r($r);
$t = unserialize('a:3:{i:0;s:1:"a";i:1;i:7;i:2;R:3;}');
$t[2]++;
print_r($t);

// Object graph with a cycle through a child and __wakeup order.
class W { public $n; public $child; function __wakeup() { echo "wakeup {$this->n}\n"; } }
$w = new W; $w->n = 'outer'; $w->child = new W; $w->child->n = 'inner'; $w->child->child = $w;
$s = serialize($w);
echo $s, "\n";
$uw = unserialize($s);
var_dump($uw->child->child === $uw, $uw->child->n);

// A reference to an object behaves like the object for numbering.
$obj = new stdClass; $ro = &$obj;
echo serialize([$obj, &$ro, $obj]), "\n";
// Property references and nested arrays.
$h = new stdClass; $h->list = [1, 2]; $h->alias = &$h->list;
echo serialize($h), "\n";
$uh = unserialize(serialize($h));
$uh->alias[] = 3;
echo count($uh->list), "\n";
// __serialize data participates in the numbering.
class S2 { public $o; function __serialize(): array { return ['a' => $this->o, 'b' => $this->o]; } function __unserialize(array $d): void { $this->o = $d; } }
$s2 = new S2; $s2->o = new stdClass;
echo serialize([$s2, $s2->o]), "\n";
$us = unserialize(serialize([$s2, $s2->o]));
var_dump($us[0]->o['a'] === $us[0]->o['b'], $us[0]->o['a'] === $us[1]);
// Malformed back-references fail.
var_dump(@unserialize('a:1:{i:0;r:5;}'), @unserialize('a:1:{i:0;R:9;}'), @unserialize('a:1:{i:0;r:1;}'));
