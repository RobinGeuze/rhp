<?php
// Property / array-element arguments of calls whose callee is only known at
// run time (methods, static methods, constructors) bind by reference when
// the parameter is by-reference (php's SEND_FUNC_ARG), else by value.
class Base { public function __construct(array &$a) { $a['base'] = 1; } }
class Child extends Base {
    public $p = [];
    public function __construct() { parent::__construct($this->p); }
}
var_dump((new Child)->p);

class Holder {
    private $c;
    public function __construct(array &$c) { $this->c =& $c; }
    public function run() { $this->c = ['x' => 2] + $this->c; }
}
class Owner {
    public $configs = ['x' => 1, 'y' => 1];
    public $ext = [];
    public function __construct() { $this->ext['h'] = new Holder($this->configs); }
    public function go() { $this->ext['h']->run(); return $this->configs; }
}
var_dump((new Owner)->go());

class V {
    public function byVal($x) { return $x; }
    public function byRef(&$x) { $x = 'set'; }
    public static function sRef(&$x) { $x = 'static'; }
    public function __call($n, $a) { return $n; }
}
$o = new V;
$arr = [];
$obj = new stdClass;
var_dump(@$o->byVal($arr['missing']), array_key_exists('missing', $arr));
var_dump(@$o->byVal($obj->nope), property_exists($obj, 'nope'));
$o->byRef($arr['k']['j']);
$o->byRef($obj->q);
V::sRef($arr['s']);
var_dump($arr, $obj->q);
var_dump(@$o->magic($arr['zz']), isset($arr['zz']));
$n = null;
var_dump($n?->byRef($arr['ns']), isset($arr['ns']));
$i = 0;
$list = [0, 0, 0];
$o->byRef($list[$i++]);
var_dump($list, $i);
try {
    new MissingClass($arr['x']);
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
var_dump(isset($arr['x']));

// Chains on temporaries / constants are always passed by value.
class K {
    const MAP = ['a' => 'const'];
    public function make() { $o = new stdClass; $o->p = 'tmp'; return $o; }
    public function show() {
        $v = new V;
        var_dump($v->byVal(self::MAP['a']));
        var_dump($v->byVal($this->make()->p));
        var_dump($v->byVal(['lit'][0]));
        var_dump($v->byVal(K::MAP['a']));
    }
}
(new K)->show();
