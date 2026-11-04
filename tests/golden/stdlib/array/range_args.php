<?php
// range()'s argument handling (php_range): step sign and size checks,
// finite bounds, character ranges and the warnings when string and number
// bounds mix.
set_error_handler(function ($no, $msg) {
    echo "  ", $no === E_DEPRECATED ? "Deprecated" : "Warning", ": $msg\n";
    return true;
});
$cases = [
    [1, 5], [5, 1], [1, 5, 2], [5, 1, 2], [1, 3, -1], [3, 1, -1], [1, 2, 3],
    [1, 1, 5], [1, 2, NAN], [1, 2, INF], [1, 2, 0], [1, 2, -0.0],
    [1, 2, PHP_INT_MIN], [0, 1, 0.25], [1.5, 4], [4, 1.5], [0, 1, 1e20],
    [1, NAN], [INF, 1], ["1", "5"], ["1.5", 3], [" 2", "4 "], ["", 3],
    ["a", "e"], ["e", "a"], ["a", "e", 2], ["a", "e", -2], ["e", "a", -2],
    ["a", "z", 30], ["A", "z", 100], ["a", 5], [5, "a"], ["a", "5"], ["5", "a"],
    ["ab", "cd"], ["a", "e", 1.5], ["1", "5", 1.5], ["a", "a"], ["z", "a", 13],
    [null, 3], [true, 3], [PHP_INT_MAX - 1, PHP_INT_MAX], [PHP_INT_MIN, PHP_INT_MAX],
    [0, 1e10, 1], [-1e10, 0],
];
foreach ($cases as $args) {
    echo "range(", implode(", ", array_map(fn($v) => var_export($v, true), $args)), ")\n";
    try {
        $r = range(...$args);
        echo "  ", json_encode($r, JSON_PRESERVE_ZERO_FRACTION), "\n";
    } catch (Throwable $e) {
        echo "  ", get_class($e), ": ", $e->getMessage(), "\n";
    }
}
