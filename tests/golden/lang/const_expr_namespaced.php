<?php
namespace NS1;
const LOCAL = 'ns-local';
const USES_GLOBAL = PHP_EOL . LOCAL;
class K { const A = LOCAL . '!'; const B = \strlen(...) ; const C = __NAMESPACE__ . '\\' . 'x'; }
echo LOCAL, USES_GLOBAL, K::A, "\n";
echo (K::B)('abcd'), " ", K::C, "\n";
echo \NS1\LOCAL, " ", \defined('NS1\LOCAL') ? 'def' : 'undef', "\n";
function f($cb = static function ($x) { return $x * 2; }) { return $cb(21); }
echo f(), "\n";
class Cl { const DOUBLE = static function (int $x): int { return $x * 2; }; }
echo (Cl::DOUBLE)(4), "\n";
try { echo UNDEFINED_LATER; } catch (\Error $e) { echo $e->getMessage(), "\n"; }
const UNDEFINED_LATER = 1;
echo UNDEFINED_LATER, "\n";
// conditional / repeated declaration does not overwrite
if (true) { echo \constant('NS1\LOCAL'), "\n"; }
class WithStaticVarMethod {
    public static function next() { static $i = self::START + 1; return $i++; }
    const START = 41;
}
echo WithStaticVarMethod::next(), WithStaticVarMethod::next(), "\n";
// a failing static initializer is retried on the next call
function flaky() { static $v = \NS1\Holder::get(); return $v; }
class Holder { public static $n = 0; public static function get() { if (self::$n++ === 0) throw new \Exception('first'); return 'second'; } }
try { flaky(); } catch (\Exception $e) { echo $e->getMessage(), "\n"; }
echo flaky(), flaky(), "\n";
// static property initializers evaluated lazily on first access
class LazyStatic { public static $v = LATE_DEF; }
\define('NS1\LATE_DEF', 'ok-late');
echo LazyStatic::$v, "\n";
// readonly class with constant-expression defaults in promoted ctor params
final class Point { public function __construct(public readonly array $xy = [K::A, LOCAL], public $o = new \ArrayObject([1])) {} }
$p = new Point(); var_dump($p->xy, count($p->o));
// enum constant expressions referencing other enums
enum Color: string { case Red = 'r'; case Green = 'g'; const FAV = self::Green; const VALS = [self::Red->value, self::Green->value]; }
var_dump(Color::FAV->name, Color::VALS);
// match is not a constant expression -> but ternary/coalesce over consts is
class Tern { const T = PHP_INT_SIZE === 8 ? 'x64' : 'x86'; const N = null ?? K::A; const ARR = ['a' => 1]['b'] ?? 'missing'; }
var_dump(Tern::T, Tern::N, Tern::ARR);
