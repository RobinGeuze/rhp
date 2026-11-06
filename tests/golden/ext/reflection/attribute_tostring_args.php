<?php
// ReflectionAttribute::__toString: folded values and the exported constant
// expression, with names as zend_compile_const_expr leaves them.
namespace Foo\Bar;
use Other\Thing;
use Other\Pkg as P;
use function Other\fn1;
use const Other\C1;
enum E { case A; const X = 1; }
#[A(1, 'a\'b\\c', "t\tn\n\x7f", [1, 'x' => 2.5], null, true, E::A, E::X, \K::Y, Thing::Z, P\Q::Z, namespace\R::Z,
  new \K(1), new Thing(), FOO, \FOO, C1, PHP_EOL, E_ALL, 1 + 2, FOO + 1, [FOO, 2], strlen(...), \strlen(...), fn1(...), Sub\f(...),
  static function () { return 1; }, name: 5, other: FOO ? 1 : 2, x: -FOO, z: FOO . 'x', w: E::class, v: __CLASS__,
  t: [1, ...FOO], s: (int) FOO, r: FOO['a'], p: E::A->name)]
function g() {}
foreach ((new \ReflectionFunction('Foo\Bar\g'))->getAttributes() as $a) echo $a;
#[NoArgs] #[EmptyArgs()]
function h() {}
foreach ((new \ReflectionFunction('Foo\Bar\h'))->getAttributes() as $a) echo $a;

class W {
    #[A(self::class, new self, __CLASS__, self::m(...), static function () {})]
    function m() {}
}
foreach ((new \ReflectionMethod(W::class, 'm'))->getAttributes() as $a) echo $a;

// A closure in a function's or method's attribute is named after it.
#[A(static function () {})]
function c() {}
class V { #[A(static function () {})] function m() {} }
echo (new \ReflectionFunction((new \ReflectionFunction('Foo\Bar\c'))->getAttributes()[0]->getArguments()[0]))->name, "\n";
echo (new \ReflectionFunction((new \ReflectionMethod(V::class, 'm'))->getAttributes()[0]->getArguments()[0]))->name, "\n";

// Default values print with smart_str_append_escaped (quotes unescaped).
function d($a = "it's\\ \n\x7f\x01", $b = ["k'\t" => 1.5, 2 => null]) {}
foreach ((new \ReflectionFunction('Foo\Bar\d'))->getParameters() as $p) echo $p, "\n";

// Heredocs and nowdocs without interpolation are folded strings, nested too.
#[A(<<<EOT
  it's
  EOT, ["a'b", <<<'N'
q'
N], "x'" . FOO)]
function n() {}
foreach ((new \ReflectionFunction('Foo\Bar\n'))->getAttributes() as $a) echo $a;
