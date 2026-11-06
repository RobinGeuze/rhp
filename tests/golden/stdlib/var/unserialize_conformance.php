<?php
// unserialize() following var_unserializer.re: error offsets, the var_hash
// shared with Serializable::unserialize(), delayed __wakeup /
// __unserialize, max_depth, the callback function, incomplete objects.
set_error_handler(function ($no, $msg) {
    echo "[$no] $msg\n";
    return true;
});

// Error offsets.
foreach ([
    'O:1:"A":1:{}', 'O:8:"stdClass":1:{s:1:"a";i:1}', 'O:8:"stdClass":-1:{}',
    'O:8:"stdClass"', 'C:8:"stdClass":2:{x}', 'C:3:"Foo":1:{x}', 'E:3:"Foo";',
    'E:5:"Foo:A";', 'a:1:{O:8:"stdClass":0:{}i:1;}', 'a:1:{a:0:{}i:1;}',
    'a:1:{R:1;i:1;}', 'r:1;', 'a:1:{i:0;r:1;}', 'a:2:{i:0;i:1;i:1;r:2;}',
    'a:1:{i:0;d:1e5;}', 'a:1:{i:0;d:.5;}', 'a:1:{i:0;d:5.;}', 'i:+5;', 'i:-0;',
    's:2:"\61b";', 'S:2:"\61b";', 'S:2:"\6xb";', 'O:0:"":0:{}',
] as $s) {
    echo json_encode($s), " => ";
    var_dump(unserialize($s));
}

// Serializable: C: on both sides; a nested serialize() / unserialize()
// shares its caller's slots.
class Box implements Serializable {
    public $v;
    function __construct($v = null) { $this->v = $v; }
    function serialize() { return serialize($this->v); }
    function unserialize($data) { $this->v = unserialize($data); }
    function __serialize(): array { return [$this->v]; }
    function __unserialize(array $d): void { $this->v = $d[0]; }
}
class Old implements Serializable {
    public $v;
    function serialize() { return serialize($this->v); }
    function unserialize($data) { $this->v = unserialize($data); }
}
$o = new stdClass;
$old = new Old;
$old->v = [$o, $o];
echo $s = serialize([$o, $old]), "\n";
$u = unserialize($s);
var_dump($u[0] === $u[1]->v[0], $u[1]->v[0] === $u[1]->v[1]);
echo serialize(new Box(1)), "\n";
var_dump(unserialize('C:3:"Box":4:{i:5;}'));
var_dump(unserialize('a:2:{i:0;s:1:"x";i:1;C:3:"Old":4:{R:2;}}'));

// R: to the root, then written by __wakeup.
class Root { public $r; function __wakeup() { $this->r = 'replaced'; } }
var_dump(unserialize('a:1:{i:0;O:4:"Root":1:{s:1:"r";R:1;}}'));

// max_depth.
var_dump(unserialize('a:1:{i:0;a:1:{i:0;a:0:{}}}', ['max_depth' => 2]));
var_dump(unserialize('a:1:{i:0;a:0:{}}', ['max_depth' => 2]));
foreach (['x', -1] as $bad) {
    try { unserialize('i:1;', ['max_depth' => $bad]); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}

// unserialize_callback_func.
function define_it($name) { echo "callback $name\n"; if ($name === 'Later') { eval('class Later { public $x; }'); } }
ini_set('unserialize_callback_func', 'define_it');
var_dump(unserialize('O:5:"Later":1:{s:1:"x";i:1;}'));
var_dump(get_class(unserialize('O:5:"Never":0:{}')));
ini_set('unserialize_callback_func', '');

// Typed properties: strict checks, int to float.
class T { public float $f; public ?T $t = null; public int $i = 0; }
var_dump(unserialize('O:1:"T":2:{s:1:"f";i:3;s:1:"t";N;}'));
try { unserialize('O:1:"T":1:{s:1:"i";s:1:"5";}'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }

// Dynamic properties.
class NoDyn { public $a; }
var_dump(unserialize('O:5:"NoDyn":2:{s:1:"a";i:1;s:1:"b";i:2;}'));

// Incomplete objects.
$inc = unserialize('O:7:"Missing":1:{s:1:"p";i:1;}');
var_dump(isset($inc->p), empty($inc->p), $inc->p ?? 'dflt', $inc->p);
foreach ([
    fn() => $inc->p = 2,
    function () use ($inc) { unset($inc->p); },
    fn() => $inc->m(),
    function () use ($inc) { $inc->p .= 'x'; },
    function () use ($inc) { $inc->q[] = 1; },
] as $f) {
    try { $f(); } catch (Error $e) { echo substr($e->getMessage(), 0, 60), "\n"; }
}
echo serialize($inc), "\n";

// SPL payloads.
$c = fn($class, $p) => 'C:' . strlen($class) . ':"' . $class . '":' . strlen($p) . ':{' . $p . '}';
print_r(unserialize($c('ArrayObject', 'x:i:0;a:1:{i:0;i:1;};m:a:1:{s:1:"m";i:2;}')));
print_r(unserialize($c('SplObjectStorage', 'x:i:1;O:8:"stdClass":0:{},s:1:"i";;m:a:0:{}')));
print_r(unserialize($c('SplStack', 'i:6;:i:1;:i:2;')));
foreach ([$c('ArrayObject', 'x:i'), $c('ArrayIterator', 'y:i:0;'), $c('SplQueue', 'i:0'), $c('ArrayObject', 'x:i:0;a:0:{};m:i:1;')] as $s) {
    try { unserialize($s); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}
// Delayed calls: in parse order, after the whole payload; a throwing
// __wakeup stops the rest.
class W {
    public $n;
    function __wakeup() { echo "wakeup {$this->n}\n"; if ($this->n === 2) { throw new Exception("stop"); } }
    function __destruct() { echo "destruct {$this->n}\n"; }
}
try {
    unserialize('a:3:{i:0;O:1:"W":1:{s:1:"n";i:1;}i:1;O:1:"W":1:{s:1:"n";i:2;}i:2;O:1:"W":1:{s:1:"n";i:3;}}');
} catch (Exception $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
echo "after\n";
echo "done\n";
