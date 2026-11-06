<?php
// The pipe operator: evaluation order, callable forms, by-reference
// parameters, traces.
function trace($label, $v) { echo "$label\n"; return $v; }
function inc(int $x): int { return $x + 1; }
function byref(&$x) { $x++; return $x; }
class M {
    public function __construct(public int $k) {}
    public function mul($x) { return $x * $this->k; }
    public static function neg($x) { return -$x; }
    public function __invoke($x) { return "inv:$x"; }
}
$m = new M(3);

var_dump(1 |> inc(...) |> inc(...));
var_dump(2 |> $m->mul(...));
var_dump(2 |> M::neg(...));
var_dump(5 |> $m);
var_dump("abc" |> 'strrev');
var_dump([3, 1, 2] |> (fn($a) => array_sum($a)));
var_dump("x" |> (function ($s) { return "$s!"; }));
var_dump(trace("lhs", 4) |> trace("rhs", inc(...)));
var_dump(-3 |> abs(...) |> \sqrt(...));

// the operand is a copy: by-reference parameters refuse it
$v = 1;
try { $v |> byref(...); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump($v);

// precedence: below arithmetic and concatenation, above comparison
var_dump(1 + 1 |> inc(...));
var_dump("a" . "b" |> strtoupper(...));
var_dump(3 |> inc(...) == 4);
var_dump(null ?? 2 |> inc(...));

// an uncaught exception's trace shows the called function, no closure
function boom($x) { throw new Exception("boom $x"); }
try { 7 |> boom(...); } catch (Exception $e) { echo str_replace(__FILE__, 'F', $e->getTraceAsString()), "\n"; }
try { 7 |> $m->mul(...) |> boom(...); } catch (Exception $e) { echo $e->getMessage(), "\n"; }

// clone(...) on the right
$o = new stdClass;
$c = $o |> clone(...);
var_dump($c == $o, $c === $o);
