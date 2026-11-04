<?php
// Lazily evaluated initializers of typed properties, static properties and
// typed class constants are type-checked (strict mode: only int -> float).
enum UE { case A; }
class A { public int $x = B::S; public static int $y = B::S; }
class C {
    public float $f = B::I;
    public static float $sf = B::I;
    public float $g = 1;
    public static float $h = 0;
    public int|float $u = 3;
    public float|string $fs = 4;
    public ?float $n = B::I;
    public ?int $z = B::N;
    public mixed $m = B::I;
}
class D {
    const int X = B::S;
    const float F = B::I;
    const float G = 2;
    const bool BB = B::T;
    const int O = B::O;
    const ?int N = B::N;
    const float|int FI = B::I;
    const UE E = B::O;
}
class P { public int $q = B::O; }
class Q { public int $w = B::N; }
class R { public static bool $b = B::I; }
class S { public int $t = B::T; }

foreach (['A' => fn() => new A, 'A::$y' => fn() => A::$y, 'P' => fn() => new P,
          'Q' => fn() => new Q, 'R::$b' => fn() => R::$b, 'S' => fn() => new S,
          'D::X' => fn() => D::X, 'D::BB' => fn() => D::BB, 'D::O' => fn() => D::O] as $k => $f) {
    try {
        var_dump($f());
    } catch (TypeError $e) {
        echo "$k: ", get_class($e), ": ", $e->getMessage(), "\n";
    }
}
class B { const S = 'str'; const I = 5; const O = UE::A; const N = null; const T = true; }
$c = new C;
var_dump(get_object_vars($c), C::$sf, C::$h);
var_dump(D::F, D::G, D::N, D::FI, D::E);
var_export([C::$sf, D::G, $c->g]);
echo "\n", json_encode([C::$h, $c->f, $c->g]), "\n";
// retried on every use, failing the same way
try { new A; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
