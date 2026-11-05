<?php
declare(strict_types=1);
namespace App;
// An unqualified in_array() in a namespace is resolved at run time and
// never folded; a qualified one is, but an unqualified constant in its
// flag names App\PHP_INT_MAX, which is not known at compile time.
$x = "a";
foreach ([
    fn() => in_array($x, ["a", "b"], 1),
    fn() => \in_array($x, ["a", "b"], 1),
    fn() => \in_array($x, ["a", "b"], true),
    fn() => \in_array($x, ["a", "b"], PHP_INT_MAX),
    fn() => \in_array($x, ["a", "b"], \PHP_INT_MAX),
] as $i => $f) {
    echo $i, ": ";
    try {
        var_dump($f());
    } catch (\Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
