<?php
// serialize()/unserialize() of objects: visibility-mangled keys, object
// identity (r:), PHP references (R:), __serialize/__unserialize,
// __sleep/__wakeup (delayed, innermost first), Serializable (C:), enums (E:),
// incomplete classes and allowed_classes.
function show($v) { echo str_replace("\0", '~', is_string($v) ? $v : var_export($v, true)), "\n"; }

class A { public $p = 1; public $q = 1; protected $r = 3; private $s = 4; }
show(serialize(new A));
show(unserialize(serialize(new A)));
show(unserialize('O:1:"A":1:{s:1:"q";i:2;}'));
show(unserialize('O:8:"stdClass":2:{s:1:"q";i:2;i:7;s:3:"int";}'));

$o = new stdClass;
$o->self = $o;
$o->list = [$o, 1];
$s = serialize([$o, $o, new A]);
show($s);
$u = unserialize($s);
var_dump($u[0] === $u[1], $u[0]->self === $u[0], $u[0]->list[0] === $u[0], $u[2] instanceof A);

class W {
    public $a = 1;
    public $log = [];
    function __sleep() { return ['a']; }
    function __wakeup() { echo "wakeup ", $this->a, "\n"; }
}
class M {
    public $x;
    public $inner;
    function __construct($x) { $this->x = $x; echo "construct $x\n"; }
    function __serialize(): array { echo "ser {$this->x}\n"; return ['xx' => $this->x, 'in' => $this->inner]; }
    function __unserialize(array $d): void { echo "unser {$d['xx']}\n"; $this->x = $d['xx']; $this->inner = $d['in']; }
}
$w = new W;
$w->a = 7;
show($s = serialize($w));
show(unserialize($s));
$m = new M(1);
$m->inner = new M(2);
$m->inner->inner = new W;
show($s = serialize($m));
$u = unserialize($s);
var_dump($u->x, $u->inner->x, $u->inner->inner->a);

class Legacy implements Serializable {
    function __serialize(): array { return ['d' => $this->d]; }
    function __unserialize(array $a): void { echo "legacy __unserialize\n"; $this->d = $a['d']; }
    public $d;
    function serialize() { return "D=" . $this->d; }
    function unserialize($data) { echo "legacy unserialize $data\n"; $this->d = substr($data, 2); }
}
class Both implements Serializable {
    public $v = 5;
    function serialize() { return "no"; }
    function unserialize($data) {}
    function __serialize(): array { return ['v' => $this->v]; }
    function __unserialize(array $a): void { $this->v = $a['v'] * 2; }
}
$l = new Legacy;
$l->d = "abc";
show($s = serialize([$l, $l]));
$u = unserialize($s);
var_dump($u[0]->d, $u[0] === $u[1]);
show($s = serialize(new Both));
var_dump(unserialize($s)->v);

enum S: string { case A = 'a'; }
enum U { case X; case Y; const C = 1; }
show(serialize(S::A));
show(serialize([U::X, U::X, U::Y]));
var_dump(unserialize(serialize(S::A)) === S::A);
var_dump(unserialize('E:3:"U:X";') === U::X);
try {
    var_dump(unserialize('O:1:"U":0:{}'));
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}

$inc = unserialize('O:7:"Missing":2:{s:1:"a";i:1;s:4:"~*~b";i:2;}');
var_dump(get_class($inc));
show(serialize(unserialize('O:7:"Missing":1:{s:1:"a";i:1;}')));
show(unserialize('O:1:"A":0:{}', ['allowed_classes' => false]));
show(unserialize('O:1:"A":0:{}', ['allowed_classes' => ['a']]));
show(unserialize('a:1:{i:0;O:1:"A":0:{}}', ['allowed_classes' => ['W']]));
try {
    unserialize('i:1;', ['allowed_classes' => 1]);
} catch (TypeError $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}

foreach ([function () {}, [1, fn() => 1], (function () { yield 1; })()] as $bad) {
    try {
        serialize($bad);
    } catch (Exception $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
class Bad { function __serialize() { return 1; } }
try {
    serialize(new Bad);
} catch (TypeError $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
class Thrower { function __wakeup() { throw new RuntimeException("no wakeup"); } }
try {
    unserialize(serialize(new Thrower));
} catch (RuntimeException $e) {
    echo "caught ", $e->getMessage(), "\n";
}

$x = 1;
$arr = [&$x, &$x, 2];
show(serialize($arr));
show(unserialize(serialize($arr)));
show(unserialize('a:2:{s:1:"1";i:1;i:5;O:8:"stdClass":0:{}}'));
