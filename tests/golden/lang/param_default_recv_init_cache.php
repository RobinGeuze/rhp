<?php ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
// RECV_INIT caches a default that is not refcounted: a deprecated constant
// deprecates on the first call only; a `new` default runs every time.
class C { #[\Deprecated] const X = 'x'; #[\Deprecated] const A = [1]; }
function f($a = C::X, $b = C::X . 'y', $c = C::A, $e = MT_RAND_PHP + 1) { return [$a, $b, $c, $e]; }
f(); f(); var_dump(f());
function g(int $i = MT_RAND_PHP . '') { return $i; }
var_dump(g(), g());
class W { function __construct() { echo "W\n"; } }
function h($w = new W) { return $w; }
h(); h();
function k($x = UNDEF_K) { return $x; }
try { k(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
define('UNDEF_K', 5);
var_dump(k(), k());
