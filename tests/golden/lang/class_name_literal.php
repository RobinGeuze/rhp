<?php
declare(ticks=1);
declare(strict_types=1);

namespace N;

// A string literal names a class as written: no namespace, one leading
// backslash dropped; self/parent/static are the scope's classes.
class P {}
class A extends P {
    const X = 'Foo'::class;
    const Y = "self"::class;
    static function m() {
        return ['Foo'::class, '\Foo\Bar'::class, 'SELF'::class, 'Parent'::class, 'static'::class, ('x y')::class];
    }
}
class B extends A {}
var_dump(A::X, A::Y, B::m());

// Other declares may come before strict_types.
function f(int $x) {}
try {
    f("1");
} catch (\TypeError $e) {
    echo str_replace(__FILE__, "F", $e->getMessage()), "\n";
}

// A closing tag after __halt_compiler() takes its newline.
var_dump(__COMPILER_HALT_OFFSET__);
__halt_compiler() ?>
data
