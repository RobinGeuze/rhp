<?php
// Writes below an ArrayAccess element (`$o['a']['b'] = v`, also via `??=`):
// php fetches the element with offsetGet for writing — a by-reference
// result is written through, an object receives the write, anything else
// is a temporary ("Indirect modification ..." notice). Internal containers
// (ArrayObject, ArrayIterator) hand out their element itself.
set_error_handler(function ($no, $str) { echo "[$no] $str\n"; return true; });
class AA implements ArrayAccess {
    public $d = [];
    function offsetExists($o): bool { echo "exists($o) "; return isset($this->d[$o]); }
    function offsetGet($o): mixed { echo "get($o) "; return $this->d[$o] ?? null; }
    function offsetSet($o, $v): void { echo "set($o) "; $this->d[$o] = $v; }
    function offsetUnset($o): void {}
}
class RefAA extends AA {
    function &offsetGet($o): mixed {
        echo "rget($o) ";
        $o ??= 'null';
        if (!isset($this->d[$o])) {
            $this->d[$o] = null;
        }
        return $this->d[$o];
    }
}
$o = new AA;
$o['w']['x'] = 1; echo "\n";
$o->d['w'] = ['x' => null];
$o['w']['x'] ??= 2; echo "\n";
var_dump($o->d);
$r = new RefAA;
$r['a']['b'] = 1; echo "\n";
$r['a']['c'] ??= 2; echo "\n";
$r[]['z'] = 3; echo "\n";
var_dump($r->d);
$holder = ['aa' => new AA];
$holder['aa']['k'] = 'v'; echo "\n";
$holder['aa']['obj'] = new stdClass; echo "\n";
$holder['aa']['obj']->p = 1; echo "\n";
$holder['aa']['obj']->q['x'] = 2; echo "\n";
print_r($holder['aa']->d['obj']);
$ao = new ArrayObject(['a' => ['b' => 1]]);
$ao['a']['c'] = 2;
$ao['n'][] = 3;
var_dump($ao->getArrayCopy());
$ai = new ArrayIterator([]);
$ai['x']['y'] = 1;
var_dump($ai->getArrayCopy());
class U { public $__x = 1; function f() {} }
print_r(new U);
