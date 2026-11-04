<?php
const A = 2;
const B = A * 3 . "x";
const C = [A, ...[1,2], 'k' => B];
const D = PHP_INT_SIZE > 4 ? 'big' : 'small';
echo B, "\n"; var_dump(C, D);
class K { const X = 'kx'; const Y = self::X . '!'; const Z = K::class; }
const E = K::Y;
echo E, " ", K::Z, "\n";
enum Suit: string { case H = 'h' . 'x'; case S = K::X; const Wild = self::S; }
var_dump(Suit::H->value, Suit::S->value, Suit::Wild);
const F = Suit::H->value;
echo F, "\n";
function f($a = A + 1, $b = K::Y, $c = Suit::S, $d = new ArrayObject([1])) { var_dump($a, $b, $c, count($d)); }
f();
f(b: 'bb');
function g() { static $s = K::X . A; static $t = [K::X => 1]; $s .= '.'; return $s; }
echo g(), g(), "\n";
class P { public $p = A . K::X; public static $q = [self::class, A]; public ?Suit $e = Suit::H; }
var_dump((new P)->p, P::$q, (new P)->e);
const N = new ArrayObject([1,2]);
var_dump(count(N));
class Q { const NOPE = UNDEF_CONST; }
try { echo Q::NOPE; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
class R { const A = self::B; const B = self::A; }
try { echo R::A; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
