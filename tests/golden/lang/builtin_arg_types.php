<?php
// zpp in coercive mode: what a builtin's scalar and array parameters accept,
// convert, deprecate and reject, by the parameter's type.

set_error_handler(function ($no, $msg) {
    echo "  ", $no === E_DEPRECATED ? "Deprecated" : "Warning", ": $msg\n";
    return true;
});

class S { function __toString(): string { return "7"; } }
$fp = fopen('php://memory', 'r');

$values = [
    'null' => null, 'true' => true, 'false' => false, '3' => 3, '2.5' => 2.5,
    '2.0' => 2.0, '-0.0' => -0.0, 'NAN' => NAN, 'INF' => INF, '1e20' => 1e20,
    '"4"' => "4", '" 4 "' => " 4 ", '"4 "' => "4 ", '"4x"' => "4x", '"x"' => "x",
    '""' => "", '"2.5"' => "2.5", '"1e3"' => "1e3", '"0x1A"' => "0x1A",
    '"1e100"' => "1e100", '[]' => [], 'S' => new S, 'stdClass' => new stdClass,
    'Closure' => fn() => 1, 'resource' => $fp,
];

$calls = [
    'int' => fn($v) => str_repeat("ab", $v),
    '?int' => fn($v) => array_slice([1, 2, 3, 4, 5], 1, $v),
    'float' => fn($v) => sqrt($v),
    'int|float' => fn($v) => abs($v),
    'number_format (zpp int|float)' => fn($v) => number_format($v),
    'bool' => fn($v) => in_array("1", [1], $v),
    'string' => fn($v) => strlen($v),
    '?string' => fn($v) => mb_strlen("abc", $v),
    'string|int' => fn($v) => array_column([[1, 2, 'k' => 'v']], $v),
    'string|int|float' => fn($v) => range($v, 5),
    'array' => fn($v) => array_sum($v),
    'array|string' => fn($v) => str_replace($v, "-", "a7b"),
    'array|int' => fn($v) => substr_replace("abcdef", "X", $v, 1),
];

// Body gaps, not zpp: number_format prints NAN, INF and huge floats its own
// way, and range() has string-range rules of its own.
$skip = [
    'number_format (zpp int|float)' => ['NAN', 'INF', '"1e100"'],
    'string|int|float' => ['"4x"', '"x"', '""', '"0x1A"'],
];

foreach ($calls as $type => $call) {
    echo "== $type\n";
    foreach ($values as $label => $v) {
        if (in_array($label, $skip[$type] ?? [], true)) {
            continue;
        }
        echo "$label: ";
        try {
            $r = $call($v);
            echo is_array($r) ? json_encode($r) : var_export($r, true), "\n";
        } catch (Throwable $e) {
            echo get_class($e), ": ", $e->getMessage(), "\n";
        }
    }
}

echo "== variadic\n";
foreach ([
    fn() => array_merge([1], 2),
    fn() => array_merge(null),
    fn() => array_intersect([1], [1], "x"),
    fn() => array_push($undefined, 1),
    fn() => max(1, []),
    fn() => sprintf("%d", []),
] as $f) {
    try {
        var_dump($f());
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}

echo "== values\n";
foreach ([
    fn() => str_repeat("x", -1),
    fn() => array_chunk([1], 0),
    fn() => range(1, 2, 0),
    fn() => array_fill(0, -1, 0),
    fn() => str_pad("x", 5, ""),
    fn() => explode("", "abc"),
    fn() => array_combine([1, 2], [1]),
    fn() => mb_strlen("x", "no-such-encoding"),
    fn() => chr(300),
    fn() => round(1.5, 0, 99),
    fn() => str_split("abc", 0),
    fn() => wordwrap("abc", 0, "", true),
] as $f) {
    try {
        var_dump($f());
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
