<?php
// Class constant fetches repeated from one site: the direct path must
// agree with the full one for every kind of constant.

set_error_handler(function ($no, $msg) { echo "diag: $msg\n"; return true; });

interface I { const IK = 'interface'; }
trait T { const TK = 'trait'; }
class A implements I {
    use T;
    const K = 'A';
    const LAZY = self::K . '+' . I::IK;
    private const P = 'private';
    protected const Q = 'protected';
    #[\Deprecated]
    const OLD = 'old';
    static function lsb() { return static::K; }
    static function priv() { return self::P; }
}
class B extends A {
    const K = 'B';
    static function prot() { return parent::Q; }
}
enum E: string {
    case X = 'x';
    const ALIAS = self::X;
}

function get($f) {
    try {
        return $f();
    } catch (Error $e) {
        return get_class($e) . ': ' . $e->getMessage();
    }
}
for ($i = 0; $i < 2; $i++) {
    echo A::K, ' ', B::K, ' ', A::lsb(), ' ', B::lsb(), ' ', A::LAZY, ' ', B::LAZY, "\n";
    echo A::IK, ' ', I::IK, ' ', A::TK, ' ', A::priv(), ' ', B::prot(), "\n";
    echo get(fn() => A::P), ' | ', get(fn() => A::Q), ' | ', get(fn() => T::TK), "\n";
    echo A::OLD, ' ', E::ALIAS->value, ' ', E::X->name, ' ', PHP_VERSION_ID > 0 ? 'ok' : '', "\n";
    echo DateTimeInterface::ATOM, ' ', ArrayObject::STD_PROP_LIST, "\n";
    foreach (['A', 'B', 'E'] as $c) {
        echo get(fn() => constant("$c::K")), ' ';
    }
    echo "\n";
}
