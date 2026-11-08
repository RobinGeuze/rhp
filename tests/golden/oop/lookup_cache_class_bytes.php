<?php
// Class names the engine resolves from strings (callables, catch of engine
// errors, class_exists/is_a, reflection) go through a cache keyed by a
// case-blind hash; every spelling must still find the right class, and an
// alias or a missing name must not borrow another's entry.
namespace Very\Long\Namespace\Path;

class Svc { public static function hi() { return __METHOD__; } }
class Svc2 extends Svc {}
class Abcdefgh { public static function hi() { return 'eight'; } }
class Abcdefghi { public static function hi() { return 'nine'; } }
class_alias(Svc::class, 'Very\Long\Namespace\Path\Alias');

$names = [
    'Very\Long\Namespace\Path\Svc', 'VERY\LONG\NAMESPACE\PATH\SVC', 'very\long\namespace\path\svc',
    '\Very\Long\Namespace\Path\Svc', 'Very\Long\Namespace\Path\Svc2', 'Very\Long\Namespace\Path\Alias',
    'very\long\namespace\path\ALIAS', 'Very\Long\Namespace\Path\Abcdefgh', 'Very\Long\Namespace\Path\ABCDEFGHI',
    'Very\Long\Namespace\Path\Missing', 'Very\Long\Namespace\Path\Sv', 'Very\Long\Namespace\Path\Svc22',
];
for ($round = 0; $round < 2; $round++) {
    foreach ($names as $n) {
        echo $n, ': ', var_export(class_exists($n), true), ' ',
            is_callable([$n, 'hi']) ? call_user_func([$n, 'hi']) : '-', ' ',
            var_export(is_a($n, 'very\long\namespace\path\svc', true), true), ' ',
            class_exists($n) ? (new \ReflectionClass($n))->getName() : '-', "\n";
    }
}
for ($i = 0; $i < 3; $i++) {
    try {
        intdiv(1, 0);
    } catch (\TypeError | \ARITHMETICERROR $e) {
        echo get_class($e), "\n";
    }
    try {
        [new Svc, 'nope']();
    } catch (\error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
