<?php
// A child redeclaring a parent's private as public / protected, three-level
// chains, visibility errors, and statics.
class A {
    private $v = 'A-private';
    private $w = 'A-w';
    public function aGet() { return $this->v; }
    public function aSet($x) { $this->v = $x; }
    public function aVars() { return get_object_vars($this); }
}
class B extends A {
    public $v = 'B-public';
    protected $w = 'B-protected';
    public function bGet() { return $this->v . '/' . $this->w; }
}
$b = new B;
echo $b->aGet(), ' ', $b->bGet(), ' ', $b->v, "\n";
$b->v = 'outside';
$b->aSet('set-by-A');
echo $b->aGet(), ' ', $b->bGet(), "\n";
var_dump($b);
var_dump($b->aVars(), get_object_vars($b));
try { echo $b->w; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { $b->w = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { unset($b->w); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(isset($b->w), property_exists($b, 'w'), property_exists('B', 'v'));

class P { private $secret = 1; protected $prot = 2; public function peek(P $o) { return $o->secret; } }
class Q extends P { public function q() { return isset($this->secret) ? 'y' : 'n'; } }
$p = new P;
try { echo $p->secret; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { echo $p->prot; } catch (Error $e) { echo $e->getMessage(), "\n"; }
$q = new Q;
echo $q->q(), "\n";
// accessing a parent's private on a child instance from outside: undefined, not an error
var_dump(isset($q->secret));
// a P method can read the private slot of any P-derived object
echo $p->peek($q), "\n";
try { echo $q->prot; } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(property_exists('Q', 'secret'), property_exists('P', 'secret'), property_exists($q, 'prot'));

// statics: a private static per class; inherited private statics stay the parent's
class S1 {
    private static $s = 'S1';
    protected static $t = 'T';
    public static function get() { return self::$s; }
    public static function lsb() { return static::$s; }
    public static function setS($v) { self::$s = $v; }
}
class S2 extends S1 {
    private static $s = 'S2';
    public static function get2() { return self::$s; }
    public static function t() { return self::$t; }
}
class S3 extends S1 {
    public static function viaParent() { return parent::get(); }
    public static function own() { return self::$s; }
}
echo S1::get(), ' ', S2::get(), ' ', S2::get2(), ' ', S2::t(), "\n";
S1::setS('changed');
echo S1::get(), ' ', S2::get2(), ' ', S3::viaParent(), "\n";
try { echo S2::lsb(), "\n"; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { echo S3::own(), "\n"; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { echo S1::$s, "\n"; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { echo S2::$t, "\n"; } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(property_exists('S3', 's'), property_exists('S2', 's'));

// closures bound to a scope see that scope's private slot
class K1 { private $z = 'k1'; }
class K2 extends K1 { private $z = 'k2'; }
$k = new K2;
$get = function () { return $this->z; };
echo Closure::bind($get, $k, K1::class)(), ' ', Closure::bind($get, $k, K2::class)(), "\n";
$set = function ($v) { $this->z = $v; };
Closure::bind($set, $k, K1::class)('k1-new');
echo Closure::bind($get, $k, K1::class)(), ' ', Closure::bind($get, $k, K2::class)(), "\n";
$vars = function () { return get_object_vars($this); };
var_dump(Closure::bind($vars, $k, K1::class)(), Closure::bind($vars, $k, K2::class)());
