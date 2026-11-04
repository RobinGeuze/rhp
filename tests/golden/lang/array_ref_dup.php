<?php
// A reference held only by the array (refcount 1) is not shared by a copy
// (zend_array_dup), e.g. the leftover of a by-ref builtin call.
function g() { $l = []; $l['x'] = [0]; sort($l['x']); $c = $l; $c['x'][0] = 5; echo $l['x'][0], $c['x'][0], "\n"; }
g();
function h() { $l = ['x' => [0]]; $r = &$l['x']; unset($r); $c = $l; $c['x'][0] = 5; echo $l['x'][0], "\n"; }
h();
$l = [];
$l['x'][0] = [];
krsort($l['x']);
$c = $l;
$c['x'][0] = [1];
var_dump(count($l['x'][0]));
// A live reference is still shared.
$a = [1, 2];
$r = &$a[0];
$b = $a;
$b[0] = 9;
var_dump($a[0]);
// Listeners collected through a by-ref sort, then copied (composer's EventDispatcher).
$listeners = ['ev' => [10 => ['a'], 0 => ['b']]];
krsort($listeners['ev']);
$copy = $listeners;
$copy['ev'][] = ['c'];
echo count($listeners['ev']), count($copy['ev']), "\n";
// Self-reference guard.
$s = [1];
$s[1] = &$s;
$t = $s;
var_dump(count($t[1]));
// A by-value read of a referenced element does not keep the reference alive.
function f($n) { $l = []; $l[$n][0] = []; krsort($l[$n]); $c = $l; $c[$n][0] = count($c[$n]); echo count($l[$n][0]), "\n"; }
f('q');
class Ed {
    public $listeners = [];
    function get($n, $s) {
        if (!isset($this->listeners[$n][0])) { $this->listeners[$n][0] = []; }
        krsort($this->listeners[$n]);
        $listeners = $this->listeners;
        $listeners[$n][0] = array_merge($listeners[$n][0], $s);
        return array_merge(...$listeners[$n]);
    }
}
$e = new Ed;
echo count($e->get('a', ['x'])), count($e->get('a', ['x'])), "\n";
$z = [null];
$zr = &$z[0];
var_dump(isset($z[0]));
