<?php
declare(strict_types=1);
// zpp in strict mode: only the declared types pass (an int widens to float),
// null is a TypeError rather than a deprecation, and Stringable objects are
// not strings. A callback a builtin calls is parsed in coercive mode.

set_error_handler(function ($no, $msg) {
    echo "  ", $no === E_DEPRECATED ? "Deprecated" : "Warning", ": $msg\n";
    return true;
});

class S { function __toString(): string { return "7"; } }

$values = [
    'null' => null, 'true' => true, '3' => 3, '2.5' => 2.5, '"4"' => "4",
    '[]' => [], 'S' => new S,
];

$calls = [
    'int' => fn($v) => str_repeat("ab", $v),
    '?int' => fn($v) => array_slice([1, 2, 3, 4, 5], 1, $v),
    'float' => fn($v) => sqrt($v),
    'int|float' => fn($v) => abs($v),
    'bool' => fn($v) => in_array("1", [1], $v),
    'string' => fn($v) => strlen($v),
    '?string' => fn($v) => mb_strlen("abc", $v),
    'string|int' => fn($v) => array_column([[1, 2, 'k' => 'v']], $v),
    'array' => fn($v) => array_sum($v),
    'array|string' => fn($v) => str_replace($v, "-", "a7b"),
];

foreach ($calls as $type => $call) {
    echo "== $type\n";
    foreach ($values as $label => $v) {
        echo "$label: ";
        try {
            $r = $call($v);
            echo is_array($r) ? json_encode($r) : var_export($r, true), "\n";
        } catch (Throwable $e) {
            echo get_class($e), ": ", $e->getMessage(), "\n";
        }
    }
}

echo "== callbacks and dynamic calls\n";
foreach ([
    fn() => array_map('strlen', [5, null]),
    fn() => array_map('str_repeat', ["a"], ["2"]),
    fn() => call_user_func('strlen', 5),
    fn() => call_user_func_array('str_repeat', ["a", "2"]),
    fn() => ('strlen')(5),
    fn() => usort($GLOBALS['undefined'], 'strcmp'),
] as $f) {
    try {
        var_dump($f());
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
