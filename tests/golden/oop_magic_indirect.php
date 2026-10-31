<?php
// Indirect modification of a magic (__get) property writes into a temporary
// (php's "has no effect" notice, silenced here) and never creates a real
// property; a by-reference __get or a returned object does receive writes.
class M {
    public $data = [];
    function __get($n) { echo "get $n\n"; return $this->data[$n] ?? null; }
    function __set($n, $v) { echo "set $n\n"; $this->data[$n] = $v; }
}
$m = new M;
$m->foo = [1];
@$m->foo[] = 2;
var_dump($m->foo);
@$m->foo['k']['j'] = 3;
$m->o = new stdClass;
$m->o->x = 5;
var_dump($m->data['o']);
@$m->cnt .= 'a';
@$m->n++;
@++$m->n;
var_dump($m->data['cnt'], $m->data['n']);
function addOne(&$arr) { $arr[] = 1; }
@addOne($m->list);
var_dump(isset($m->data['list']));
print_r($m);
echo "\n";

class R {
    private $store = ['a' => [1]];
    function &__get($n) { echo "rget $n\n"; return $this->store[$n]; }
    function dump() { print_r($this->store); }
}
$r = new R;
$r->a[] = 2;
$r->b['x'] = 'new';
$r->dump();

// A private property seen from outside goes through __get as well.
class P {
    private $hidden = ['v' => 1];
    function __get($n) { echo "pget $n\n"; return $this->hidden; }
}
$p = new P;
@$p->hidden['v'] = 99;
print_r($p);
echo "\n";
// Without __get, a nested write to a missing property creates it.
$s = new stdClass;
$s->arr[] = 1;
$s->deep->x ?? null;
print_r($s);
echo "\n";
