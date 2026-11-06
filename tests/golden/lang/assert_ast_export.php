<?php
// assert()'s message: php's AST export of the expression.
function t(callable $f): void
{
    try {
        $f();
    } catch (AssertionError $e) {
        echo $e->getMessage(), "\n";
    }
}

t(fn() => assert(-1 > 0));
t(fn() => assert(0x10 > 100 && 1e3 == 2 && 1.0 == 2 && 0.1 + 0.2 == 5 && 1e100 == 3 && -0.0 == 1));
t(fn() => assert(PHP_INT_MAX + 1 == 1.5e-7 && 123456789012345678 == 0.30000000000000004));
t(fn() => assert(false && ('a\'b\\c' == "x\ny\$" && "a$b{$c}d" && TRUE && \null && namespace\foo())));
t(fn() => assert(<<<EOT
  hello
  EOT === <<<'X'
  w
  X));
t(fn() => assert(false && (__LINE__ == __FILE__ && __CLASS__ && array(1, 2) == [3, 'a' => &$x, ...$y] && list($a, , $b) = [1])));
t(fn() => assert(false && ($a->b?->c::$d::E[0][1] ?? null)));
t(fn() => assert(false && (!($a = 1) || (int)$x . (string)$y || @f() || clone $x || new Foo || new Foo(1) || new $c || new (foo()) || $x instanceof Foo || print 1)));
t(fn() => assert(false && (isset($a, $b[1]) && empty($c) && $x++ && --$y && ~$z && +$q && -$r && (1 + 2) * 3 && 1 + (2 * 3) && 2 ** 3 ** 4 && (2 ** 3) ** 4 && -2 ** 2 && $a = $b = 3)));
t(fn() => assert(false && (($a ? $b : $c) ? 1 : 2)));
t(fn() => assert(false && ($a ?: $b && $x ?? $y ?? $z && ($x ?? $y) ?? $z)));
t(fn() => assert(false && (Foo::bar() && Foo::$baz && Foo::CLASS && $x::class && static::X && self::f(...) && $o->m(...) && strlen(...) && $$x && ${'a b'} && $o->{'a b'} && $o->$p && $o::{$m}())));
t(fn() => assert(false && (f(a: 1, b: 2) && include 'x.php')));
t(fn() => assert(false && (exit(1) && die("x") && exit && eval('1;'))));
t(fn() => assert(false && ($a and $b or $c xor $d) && $x <> 1 && $y <=> 2 && $s .= 'x' && $n ??= [] && $r = &$q));
t(fn() => assert(false && function () {
    $a = (yield) && (yield 1 => 2) && (yield from $g) && throw new Exception('x');
    yield;
}));
t(fn() => assert(false && "tab\there\x00" === `cmd\`q`));
t(fn() => assert((fn() => false)()));
t(fn() => assert(false && (fn&(int ...$a): ?int => $a) && static fn() => 1));
t(fn() => assert(false && function () {
    echo 1, 2;
    ?>inline<?php
    global $a, $b;
    unset($a, $b);
    static $s = [1], $t;
    for (;;) {
        break;
    }
    foreach ($a as [$k, $v]) continue 2;
    while (true);
    if ($a): echo 1; elseif ($b): echo 2; else: echo 3; endif;
    switch ($a) { case 1: case 2: echo 3; default: }
    try { } catch (A|B) { } finally { }
    function inner(&$r = null, string|int|null ...$rest): never { throw new E; }
    interface I extends J, K { const C = 1; function m(); }
    trait T { abstract protected static function a(); }
    enum E: string implements I { case A = 'a'; }
    final readonly class C extends P { var $v; private static ?int $s = 1, $t; }
    return match (true) { default => 1 };
}));
