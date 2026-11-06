<?php
// Behaviour around property writes that the phpt corpus does not pin.

class RO {
    public public(set) readonly int $open;
    public readonly int $implicit;
    public function __construct() { $this->implicit = 1; }
    public function __clone() { $this->helper(); }
    private function helper() { $this->implicit = 2; }
}
$r = new RO;
$r->open = 5; // public(set) readonly: writable from anywhere, once
var_dump($r->open);
try { $r->open = 6; } catch (Error $e) { echo $e->getMessage(), "\n"; }
$c = clone $r; // a method called from __clone may reinitialize too
var_dump($c->implicit);
try { $c->__clone(); } catch (Error $e) { echo $e->getMessage(), "\n"; }

// Static properties reached through ->: silent with __get / __set.
class S {
    public static $s = 1;
    public function __get($n) { return "get $n"; }
}
$s = new S;
var_dump($s->s);

// A nested write fetch creates the missing dynamic property.
$o = new stdClass;
try { $o->a->b = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump($o);
try { (function () { return null; })()->x->y = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }

// Typed statics: coercion, overflow, uninitialized reads.
class T {
    public static int $i = PHP_INT_MAX;
    public static ?string $u;
}
var_dump(T::$i - 1 + (T::$i = "5"));
T::$i = PHP_INT_MAX;
try { T::$i++; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { T::$u .= "x"; } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(isset(T::$u));
T::$u = 1;
var_dump(T::$u);

// Compound assignment and ++ coerce into typed properties.
class P { public int $i = 1; public string $s = "a"; public ?float $f = null; }
$p = new P;
$p->i .= "2";
$p->s .= 3;
$p->f++;
var_dump($p);
$x = ($p->i += "4");
var_dump($x);

// A by-reference __get result is copied for $o->p++.
class M {
    private $v = 1;
    public function &__get($n) { return $this->v; }
    public function __set($n, $val) { $this->v = $val; }
}
$m = new M;
var_dump($m->p++, $m->p);

// Enum name/value are declared readonly properties.
enum E: string { case A = 'a'; }
var_dump(property_exists(E::A, 'value'), (new ReflectionProperty(E::class, 'name'))->isReadOnly());
$ea = E::A;
try { $v = &$ea->name; } catch (Error $e) { echo $e->getMessage(), "\n"; }
foreach ((object) ["\0X\0y" => 1, "\0*\0z" => 2] as $k => $v) { var_dump($k); }
