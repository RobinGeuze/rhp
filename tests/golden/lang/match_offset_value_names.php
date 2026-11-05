<?php
// How engine errors name a value: UnhandledMatchError prints a scalar or an
// enum case as smart_str_append_zval does (strings quoted, escaped and cut
// at 15 bytes), else "of type" and zend_zval_type_name; an illegal offset
// names its type with zend_zval_type_name, an anonymous class's name cut
// at its NUL byte.
enum Suit { case Hearts; }
enum Num: int { case One = 1; }
function t(callable $c) {
    try {
        $c();
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
$subjects = [
    1, -5, true, false, null, 1.5, 2.0, 1e100, -INF, NAN, "x",
    "a long string over fifteen", "tab\there\n", "\xff\x00", [],
    new stdClass, STDIN, function () {}, Suit::Hearts, Num::One, new class {},
];
foreach ($subjects as $v) {
    t(fn() => match ($v) { 'zz' => 1 });
}
$anon = new class {};
$a = [];
t(function () use ($a, $anon) { $a[$anon] = 1; });
t(function () use ($a, $anon) { $a[$anon][] = 1; });
t(function () use ($a, $anon) { unset($a[$anon]); });
t(function () use ($a, $anon) { $x = $a[$anon]; });
t(function () use ($anon) { $s = "abc"; $s[$anon] = "x"; });
t(function () use ($anon) { $s = "abc"; $x = $s[$anon]; });
t(function () use ($a) { $a[new stdClass] = 1; });
t(function () use ($a) { $a[[]] = 1; });
t(function () use ($a) { unset($a[function () {}]); });
