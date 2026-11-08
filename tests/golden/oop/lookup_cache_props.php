<?php
// One property site reached with several classes and scopes: the lookup
// caches must give what a fresh lookup gives every time.

set_error_handler(function ($no, $msg) { echo "diag: $msg\n"; return true; });

class P {
    private $x = 'P private';
    public $y = 'P public';
    function readX($o) { return $o->x; }
    function readY($o) { return $o->y; }
}
class C extends P {
    public $x = 'C public';
    public $y = 'C public y';
}
class D extends P {}
class Q { public $x = 'Q public'; }

$p = new P; $c = new C; $d = new D; $q = new Q;
for ($i = 0; $i < 3; $i++) {
    // From P's scope: P's private wins over C's public redeclaration.
    echo $p->readX($p), ' | ', $p->readX($c), ' | ', $p->readX($d), ' | ', $p->readX($q), "\n";
    echo $p->readY($p), ' | ', $p->readY($c), "\n";
}
function outside($o) {
    try {
        return $o->x;
    } catch (Error $e) {
        return get_class($e) . ': ' . $e->getMessage();
    }
}
foreach ([$c, $p, $q, $d, $c] as $o) {
    echo outside($o), "\n";
}

// Unset, then __get; set again, then a plain read.
class M {
    public $a = 1;
    function __get($n) { return "__get($n)"; }
}
$m = new M;
for ($i = 0; $i < 4; $i++) {
    if ($i == 1) unset($m->a);
    if ($i == 3) $m->a = 'back';
    echo $m->a, ' ', isset($m->a) ? 'set' : 'unset', ' ', $m->a ?? 'null', "\n";
}

// Typed properties: same-kind writes, coercion, overflow, false type.
class T {
    public int $i = PHP_INT_MAX - 1;
    public ?string $s = null;
    public float $f = 1.5;
    public int|string $u = 1;
    public false|int $b = false;
    public readonly int $r;
    function __construct() { $this->r = 1; }
}
$t = new T;
for ($k = 0; $k < 3; $k++) {
    try {
        $t->i++;
        echo "i=", var_export($t->i, true), "\n";
    } catch (TypeError $e) {
        echo $e->getMessage(), "\n";
    }
}
$t->i = 5;
$t->i += 2;
$t->i .= '1';
var_dump($t->i);
$t->s = 'a';
$t->s .= 'b';
$t->s = null;
var_dump($t->s);
$t->f += 1;
$t->f = 2;
var_dump($t->f);
$t->u = 'x';
$t->u = 3;
$t->u .= '';
var_dump($t->u);
try {
    $t->b = true;
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
$t->b = 4;
$t->b++;
var_dump($t->b);
for ($k = 0; $k < 2; $k++) {
    try {
        $t->r++;
    } catch (Error $e) {
        echo $e->getMessage(), "\n";
    }
}

// Asymmetric visibility and hooks are never taken from the cache.
class V {
    public private(set) int $n = 0;
    public int $h = 0 {
        set(int $v) { echo "hook $v\n"; $this->h = $v * 10; }
    }
    function bump() { $this->n++; $this->n += 2; return $this->n; }
}
$v = new V;
for ($k = 0; $k < 2; $k++) {
    echo $v->bump(), "\n";
    try {
        $v->n++;
    } catch (Error $e) {
        echo $e->getMessage(), "\n";
    }
    $v->h = $k;
    $v->h += 1;
    echo $v->h, "\n";
}

// A reference in the slot.
class R { public $a = 1; public int $t = 1; }
$r = new R;
$ref = &$r->a;
$tref = &$r->t;
for ($k = 0; $k < 2; $k++) {
    $r->a++;
    $r->a .= 'x';
    try {
        $r->t += 1;
        $r->t = 'nope';
    } catch (TypeError $e) {
        echo $e->getMessage(), "\n";
    }
    var_dump($ref, $tref);
}
