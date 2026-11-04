<?php $o=new stdClass; $x=1; $o->r=&$x; unset($x); $o2=clone $o; $o2->r=2; var_dump($o->r);
$o=new stdClass; $o->r=1; $y=&$o->r; unset($y); $o2=clone $o; $o2->r=2; var_dump($o->r);
function f(&$a){ $a++; } $p=new stdClass; $p->q=1; f($p->q); $p2=clone $p; $p2->q=10; var_dump($p->q);
$k=new stdClass; $k->list=[1]; $tmp=&$k->list; unset($tmp); $k2=clone $k; $k2->list[]=2; var_dump(count($k->list));
function g(){ $o=new stdClass; $x=1; $o->r=&$x; unset($x); $o2=clone $o; $o2->r=2; var_dump($o->r); $o->s=1; $z=&$o->s; $o3=clone $o; $o3->s=5; var_dump($o->s); }
g();
$a=new stdClass; $v=1; $a->r=&$v; $c=clone $a; $c->r=3; var_dump($v, $a->r);
// A reference shared with a live variable stays shared in the clone.
class H { public $r; public $plain = 1; }
$h = new H; $live = 'live'; $h->r = &$live;
$hc = clone $h; $hc->r = 'changed'; $hc->plain = 2;
var_dump($live, $h->r, $h->plain);
$arr = [1]; $h2 = new H; $h2->r = &$arr; $h3 = clone $h2; $h3->r[] = 2; var_dump(count($arr));
