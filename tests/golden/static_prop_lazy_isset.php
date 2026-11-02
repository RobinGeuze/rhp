<?php
// isset() / ?? / empty() on a static property with a non-literal initializer
// run the class's deferred initializers first.
class V
{
    protected static $v = ['a' => [self::class, 'x'], 'b' => 1];
    public static function t()
    {
        var_dump(isset(static::$v['a']), static::$v['b'] ?? 'dflt', empty(self::$v));
    }
}
V::t();

class W { public static $w = W::class . '!'; }
var_dump(isset(W::$w), W::$w ?? 0);

class X2 { public static $x = [PHP_INT_SIZE > 0]; }
var_dump(empty(X2::$x), X2::$x[0] ?? 'none');

class Y2 { private static $hidden = Y2::class; }
var_dump(isset(Y2::$hidden), Y2::$hidden ?? 'inaccessible');
