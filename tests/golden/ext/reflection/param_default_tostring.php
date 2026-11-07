<?php
// ReflectionParameter::__toString() prints a default php keeps as a
// constant AST as zend_ast_export does (constants are never substituted in
// parameter defaults), and a lone `iterable` as Traversable|array.
namespace N { interface I { const X = 1; } }
namespace M {
use N\I;
enum E { case A; }
class C { const Y = 2; function __construct(...$a) {} }
function f(iterable $a = [], ?iterable $b = null, iterable|int $c = 1, $d = I::X, $e = new \ArrayObject(), $f = PHP_EOL, $g = \PHP_VERSION, $h = E::A, $i = [I::X, 'a' => C::class ?? 1], $j = C::Y + 1, $k = FOO, $l = \strlen(...), $m = 1.0, $n = "a\nb", $o = M_PI, $q = __LINE__, $s = null, $t = [1, [2]], $u = C::class, $v = -1, $w = !true, $x = ~1, $z = new C(1, b: 2)): iterable { return []; }
class K {
    public iterable $p = [];
    public ?iterable $q = null;
    function m(self $a = new self, int $b = self::Y, $c = [self::Y => E::A]) {}
}
$show = fn ($r) => print(preg_replace('/^  @@.*\n/m', '', (string) $r));
$show(new \ReflectionFunction('M\f'));
$show(new \ReflectionMethod('M\K', 'm'));
echo new \ReflectionProperty('M\K', 'p'), new \ReflectionProperty('M\K', 'q');
echo (new \ReflectionFunction('M\f'))->getParameters()[0]->getType(), "\n";
}
namespace {
function g($f = PHP_EOL, $j = E_ALL | E_USER_ERROR, $l = [PHP_INT_MAX => 1], $n = true, $o = NULL) {}
echo preg_replace('/^  @@.*\n/m', '', (string) new ReflectionFunction('g'));
// Literal-only parts php folds at compile time print as the value.
function h($a = 'abc'[0], $b = [1, 2][1], $c = true ? 1 : 2, $d = 1 ?: 2, $e = [...[1, 2]], $f = ['a' => 1]['a'], $g = 2 ** 3 . 'x') {}
foreach ((new ReflectionFunction('h'))->getParameters() as $p) echo $p, "\n";
}
