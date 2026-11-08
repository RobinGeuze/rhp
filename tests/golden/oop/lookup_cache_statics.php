<?php
// Static property reads repeated from one site: the cached path must
// agree with the full one (visibility, inheritance, traits, typed
// properties left uninitialized, initializers that run on first use).

set_error_handler(function ($no, $msg) { echo "diag: $msg\n"; return true; });

const BASE = 10;
trait T { public static $t = 'trait'; }
class A {
    use T;
    public static $pub = 'A::pub';
    protected static $prot = 'A::prot';
    private static $priv = 'A::priv';
    public static $lazy = BASE * 2;
    public static int $typed;
    public static $map = ['a' => 1];
    static function priv() { return self::$priv; }
    static function prot() { return static::$prot; }
    static function lazy() { return self::$lazy; }
}
class B extends A {
    protected static $prot = 'B::prot';
    static function parentPriv() { return parent::$priv; }
}
class C { public static $x = 1; }

function get($f) {
    try {
        return $f();
    } catch (Error $e) {
        return get_class($e) . ': ' . $e->getMessage();
    }
}
for ($i = 0; $i < 3; $i++) {
    echo A::$pub, ' ', B::$pub, ' ', A::priv(), ' ', A::prot(), ' ', B::prot(), ' ', A::lazy(), "\n";
    echo get(fn() => A::$priv), ' | ', get(fn() => A::$prot), ' | ', get(fn() => B::parentPriv()), "\n";
    echo get(fn() => A::$typed), ' | ', get(fn() => A::$nope), ' | ', A::$t, ' ', get(fn() => T::$t), "\n";
    echo isset(A::$map['a']) ? 'set' : 'unset', ' ', isset(A::$priv) ? 'set' : 'unset', ' ', isset(A::$typed) ? 'set' : 'unset', ' ', A::$map['a'] ?? 'none', "\n";
    A::$map['a']++;
    B::$pub .= '+';
    C::$x *= 2;
    if ($i == 1) {
        A::$typed = 5;
    }
    echo C::$x, ' ', A::$pub, "\n";
}
$r = &C::$x;
$r = 'ref';
echo C::$x, ' ', A::$typed, "\n";
