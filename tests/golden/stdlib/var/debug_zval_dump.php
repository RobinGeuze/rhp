<?php
// debug_zval_dump(): refcounts, interned literals, packed arrays and
// references (php_debug_zval_dump).
debug_zval_dump(1, 1.5, true, null, "", "x", "literal");
debug_zval_dump([], [1, 2], ['k' => 'v'], [5 => 'a'], [100 => 'a']);

function f() {
    $s = str_repeat('ab', 3);
    debug_zval_dump($s);
    $t = $s;
    debug_zval_dump($s);
    $o = new stdClass;
    $o->self = $o;
    $o->list = [1, $s];
    debug_zval_dump($o);
    $p = $o;
    debug_zval_dump($p);
    $r = 1;
    $arr = [&$r, &$r];
    debug_zval_dump($arr);
    unset($r);
    $arr2 = [&$arr[0]];
    debug_zval_dump($arr2);
}
f();

$x = 1;
$refs = [&$x];
debug_zval_dump($refs);
unset($x);
debug_zval_dump($refs);

enum E { case A; }
function g() { debug_zval_dump(E::A); }
g();
debug_zval_dump(STDIN);

class Dbg {
    public int $typed;
    public $plain = 'p';
    public function __debugInfo() { return ['shown' => 1]; }
}
class Plain {
    public int $typed;
    protected $prot = 1;
    private $priv = 2;
}
debug_zval_dump(new Dbg, new Plain);

// two global names bound to one reference
$ga = 1;
$gb = &$ga;
$garr = [&$ga];
debug_zval_dump($garr);
