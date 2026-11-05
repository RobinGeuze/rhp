<?php
// lazy evaluation: class constant referencing a constant defined later at runtime
class L { const A = LATER . '!'; public $p = LATER; public static $s = [LATER]; }
define('LATER', 'late');
echo L::A, "\n";
var_dump((new L)->p, L::$s);
// inheritance of pending constants
class Base { const X = self::Y . 'b'; const Y = 'y'; public $q = self::X; }
class Child extends Base { const Z = parent::X . 'c'; const Y = 'child'; }
echo Child::X, " ", Child::Z, " ", Base::X, "\n";
var_dump((new Child)->q);
// interface constants
interface I { const IC = 'i' . PHP_EOL; }
class Impl implements I { const D = self::IC . 'x'; }
echo Impl::D, "\n";
// undefined class constant inside an initializer
class U { const A = Nope::B; const OK = 1; }
echo U::OK, "\n";
try { echo U::A; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { echo U::A; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
class U2 { const A = self::NOPE; }
try { echo U2::A; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
// self-referencing
class S1 { const A = self::A; }
try { echo S1::A; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
class S2 { const A = S3::B; } class S3 { const B = S2::A; }
try { echo S2::A; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
// failing property default on instantiation; retried each time
class PF { public $x = UNDEF_P; }
try { new PF; } catch (Error $e) { echo $e->getMessage(), "\n"; }
define('UNDEF_P', 5);
var_dump((new PF)->x);
// operators in constant expressions
class Ops {
    const A = 1 << 3 | 1;
    const B = 7 % 3 ** 2;
    const C = !true ?: 'elvis';
    const D = null ?? 'coalesce';
    const E = [1, 2, 3][1];
    const F = ['a' => ['b' => 'deep']]['a']['b'];
    const G = 'abc'[1];
    const H = -PHP_INT_MAX - 1;
    const I = 1 <=> 2;
    const J = true xor false;
    const K = 10 / 4;
    const M = ~5;
    const N = 'x' . 1.5 . true . null;
    const O = PHP_VERSION_ID >= 80000 && PHP_OS_FAMILY !== '';
    const P = __LINE__;
    const Q = __CLASS__;
    const R = [...self::ARR, ...['k' => 'v'], 9];
    const ARR = [1, 2];
    const S = self::class;
    const T = M_PI > 3;
}
var_dump(Ops::A, Ops::B, Ops::C, Ops::D, Ops::E, Ops::F, Ops::G, Ops::H, Ops::I, Ops::J, Ops::K, Ops::M, Ops::N, Ops::O, Ops::P, Ops::Q, Ops::R, Ops::S, Ops::T);
// enum in const exprs
enum Status: int { case On = 1 << 0; case Off = self::BASE + 1; const BASE = 10; const DEFAULT = self::On; }
var_dump(Status::Off->value, Status::DEFAULT, Status::from(11), Status::tryFrom(1), count(Status::cases()));
class UsesEnum { const V = Status::On->value; const N = Status::Off->name; public Status $s = Status::Off; }
var_dump(UsesEnum::V, UsesEnum::N, (new UsesEnum)->s);
enum Pure { case A; case B; const ALIAS = self::B; }
var_dump(Pure::ALIAS, Pure::ALIAS === Pure::B);
// constant()/defined() with lazy consts
var_dump(constant('Ops::A'), defined('Ops::B'), constant('Status::DEFAULT'));
// static vars with non-constant initializers
function counter() { static $n = Ops::A * 2; return ++$n; }
echo counter(), counter(), counter(), "\n";
function withNew() { static $o = new ArrayObject([1, 2, 3]); return count($o); }
echo withNew(), withNew(), "\n";
function callInit() { static $c = strtoupper('abc'); static $d; return $c . var_export($d, true); }
echo callInit(), "\n";
class SV { public function m() { static $x = self::class . '/' . __FUNCTION__; return $x; } }
echo (new SV)->m(), "\n";
// global const forms
const G1 = Ops::A + 1, G2 = G1 * 2;
const G3 = new stdClass;
const G4 = Status::On;
var_dump(G1, G2, get_class(G3), G4);
// param defaults
function pd($a = Ops::ARR, $b = G1 . '-', $c = new ArrayObject(), $d = Status::Off, $e = PHP_INT_SIZE * 2) { return [$a, $b, get_class($c), $d, $e]; }
var_dump(pd());
var_dump(pd(e: 1));
var_dump(pd(1, c: new ArrayIterator()));
// trait constant + static
trait T1 { public static $ts = self::class; const TC = self::class . '!'; }
class UT1 { use T1; }
class UT2 { use T1; }
var_dump(UT1::$ts, UT2::$ts, UT1::TC);
