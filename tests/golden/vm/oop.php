<?php
// Closures, objects, exceptions across frames and enums.

// closure_captures_by_value_at_definition
$x = 10;
$f = function () use ($x) { return $x; };
$x = 20;
echo $f(), "\n";

// nested_closures_capture_outer
function make($n) {
    return function ($x) use ($n) {
        return function ($y) use ($n, $x) { return $n + $x + $y; };
    };
}
$f = make(1);
$g = $f(2);
echo $g(3), "\n";

// private_property_access_blocked_outside
class Priv { private $s = 'secret'; }
try {
    echo (new Priv())->s;
} catch (Throwable $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}

// constructor_property_promotion: a promoted parameter's default
class P { public function __construct(public int $x, public int $y = 5) {} }
$p = new P(3); echo $p->x, ',', $p->y, ' ';
$q = new P(1, 2); echo $q->x, ',', $q->y, "\n";

// instanceof_class_and_interface
interface I { const V = 42; }
class C implements I {}
class D {}
$c = new C();
var_dump($c instanceof I, $c instanceof C, new C() instanceof D);

// interface_constant_access
echo I::V, ' ', C::V, "\n";

// magic_tostring_in_contexts
class M {
    public function __construct(public int $v) {}
    public function __toString() { return 'M(' . $this->v . ')'; }
}
$m = new M(7);
echo $m, ' ', "val=$m", ' ', (string)$m, "\n";

// nullsafe_operator
class Node { public ?Node $next = null; public int $v = 0; }
$a = new Node(); $a->v = 1;
echo $a->next?->v ?? 'none', ' ';
$a->next = new Node(); $a->next->v = 9;
echo $a->next?->v, "\n";

// exception_unwinds_across_frames
function deep() { throw new RuntimeException('boom'); }
function mid() { deep(); }
try {
    mid();
} catch (Exception $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}

// backed_enum_value_from_tryfrom: an int-backed enum
enum Code: int { case Ok = 200; case NotFound = 404; }
echo Code::Ok->value, ' ', Code::from(404)->name, "\n";
var_dump(Code::tryFrom(999));

// backed_enum_from_invalid_throws_value_error
try {
    Code::from(99);
} catch (Throwable $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}

// enum_var_dump_shape_and_interfaces
enum S: string { case A = 'a'; }
enum U { case A; }
var_dump(S::A, S::A instanceof UnitEnum, S::A instanceof BackedEnum, U::A instanceof BackedEnum);
