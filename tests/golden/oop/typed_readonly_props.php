<?php
// Typed properties start uninitialized, are coerced/checked on assignment;
// readonly properties are enforced (php 8.4 rules, __clone reinit).
class U { public int $a; public ?string $b; public $c; private int $d; public int $e = 3; }
$u = new U;
var_dump($u);
echo serialize($u), "\n", json_encode($u), "\n", count((array) $u), "\n";
var_dump(get_object_vars($u));
$u->a = "5"; unset($u->e);
var_dump($u);
function t(callable $f) { try { $f(); echo "ok\n"; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; } }
t(fn() => print($u->e));
class P { public string $s; public int $i; public ?array $a; public P $self; public float $f; public int|string $is; }
$p = new P;
t(function () use ($p) { $p->s = new stdClass; });
t(function () use ($p) { $p->s = 5; });
t(function () use ($p) { $p->i = "5"; });
t(function () use ($p) { $p->i = "x"; });
t(function () use ($p) { $p->self = new stdClass; });
t(function () use ($p) { $p->f = 3; });
t(function () use ($p) { $p->a = null; });
t(function () use ($p) { $p->is = 1.0; });
t(function () use ($p) { $p->s = null; });
class T { function __construct(public $v) {} function __toString(): string { return $this->v; } }
$p->s = new T('x');
print_r($p); echo "\n";
class RO2 {
    public readonly int $i;
    public function __construct(public readonly array $v = []) { $this->i = 1; }
    function bump() { $this->i = 2; }
    function __clone() { $this->i++; }
}
$r = new RO2;
t(function () use ($r) { $r->i = 5; });
t(fn() => $r->bump());
t(function () use ($r) { unset($r->i); });
t(function () use ($r) { $r->v[] = 3; });
t(function () use ($r) { $x = &$r->v; });
t(function () use ($r) { foreach ($r as &$q) {} });
$c = clone $r;
print_r($c); echo "\n";
class A { protected readonly int $prop; public function __construct() { $this->prop = 42; } }
class B extends A { public readonly int $prop; }
var_dump((new B)->prop);
class Outside { public readonly int $x; }
t(function () { $o = new Outside; $o->x = 1; });
enum Status: int { case On = 1; }
t(function () { $y = Status::On; $y->value = 3; });
t(fn() => clone Status::On);
$f = function () { return 1; }; $g = clone $f; var_dump($g(), $g === $f);
