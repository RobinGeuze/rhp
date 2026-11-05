<?php
declare(strict_types=1);
// php compiles in_array() with a constant array of strings or ints (only
// non-numeric strings when not strict) and a literal or compile-time
// constant flag to ZEND_IN_ARRAY, which reads the flag with zend_is_true
// and never type-checks it. Other shapes are calls, whose zpp throws in a
// strict_types file.
$x = "a";
$cases = [
    'int flag' => fn() => in_array($x, ["a", "b"], 1),
    'string flag' => fn() => in_array($x, ["a", "b"], "yes"),
    'float flag' => fn() => in_array($x, ["a", "b"], 1.5),
    'parenthesized' => fn() => in_array($x, ["a", "b"], (0)),
    'builtin constant' => fn() => in_array($x, ["a", "b"], PHP_INT_MAX),
    'E_ALL' => fn() => in_array($x, ["a", "b"], E_ALL),
    'NAN' => fn() => in_array($x, ["a", "b"], NAN),
    'null' => fn() => in_array($x, ["a", "b"], null),
    'NULL any case' => fn() => in_array($x, ["a", "b"], nUlL),
    'qualified' => fn() => \in_array($x, ["a", "b"], 1),
    'mixed keys' => fn() => in_array($x, ["k" => "a", "b"], 1),
    'strict ints and strings' => fn() => in_array($x, ["a", 1], 1),
    'strict numeric string' => fn() => in_array($x, ["1", "b"], 1),
    'empty haystack' => fn() => in_array($x, [], 0),
    'loose ints' => fn() => in_array($x, [1, 2], 0),
    'loose numeric string' => fn() => in_array($x, ["1", "b"], 0),
    'float element' => fn() => in_array($x, [1.5], 1),
    'variable element' => fn() => in_array($x, [$x], 1),
    'variable flag' => fn() => in_array($x, ["a"], $x),
    'negative flag' => fn() => in_array($x, ["a", "b"], -1),
    'expression flag' => fn() => in_array($x, ["a", "b"], true ? 1 : 0),
    'undefined constant' => fn() => in_array($x, ["a"], NO_SUCH_CONSTANT),
    'spread' => fn() => in_array(...[$x, ["a", "b"], 1]),
];
foreach ($cases as $name => $f) {
    echo $name, ": ";
    try {
        var_dump($f());
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
