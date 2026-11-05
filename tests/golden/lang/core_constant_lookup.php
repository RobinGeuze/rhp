<?php
// constant()/defined() with self::/parent::/static::, visibility from the
// calling scope, private constants of a parent, define() checks.

class A {
    const PUB = 'a-pub';
    protected const PROT = 'a-prot';
    private const PRIV = 'a-priv';

    public static function probe() {
        foreach (['self::PRIV', 'static::PRIV', 'static::PROT', 'static::OWN', 'parent::X'] as $n) {
            try {
                var_dump($n, defined($n), constant($n));
            } catch (Error $e) {
                echo $n, ': ', $e->getMessage(), "\n";
            }
        }
    }
}
class B extends A {
    const OWN = 'b-own';
    public static function inside() {
        var_dump(defined('A::PROT'), constant('A::PROT'));
        var_dump(defined('A::PRIV'));
        try {
            constant('A::PRIV');
        } catch (Error $e) {
            echo $e->getMessage(), "\n";
        }
        try {
            var_dump(static::PRIV);
        } catch (Error $e) {
            echo $e->getMessage(), "\n";
        }
    }
}
A::probe();
B::probe();
B::inside();
var_dump(array_keys((new ReflectionClass('B'))->getConstants()));

foreach (['A::PROT', 'self::X', 'Missing::X', '\A::PUB', 'A::NOPE'] as $n) {
    try {
        var_dump($n, defined($n), constant($n));
    } catch (Error $e) {
        echo $n, ': ', $e->getMessage(), "\n";
    }
}

trait T { const TC = 1; }
try {
    constant('T::TC');
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
var_dump(defined('T::TC'));

enum Suit { case Hearts; }
var_dump(constant('Suit::Hearts'));

$a = [1, [2, 3]];
$r = &$a[1];
var_dump(define('ARR', $a), ARR);
$rec = [1];
$rec[] = &$rec;
try {
    define('REC', $rec);
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
try {
    define('A::B', 1);
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
