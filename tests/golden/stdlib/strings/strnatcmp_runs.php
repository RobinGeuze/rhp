<?php
// strnatcmp_ex: after an equal run of digits the next characters are
// compared before whitespace is skipped again.
$pairs = [
    ["1.0 || 2.0 || 3.0", "1.0|2.0|3.0"],
    ["v1.0 beta", "v1.0beta"],
    ["A 1", "a1"],
    ["x10 y", "x10y"],
    ["x10y", "x10 y"],
    ["12 ", "12"],
    ["12a", "12b"],
    ["007 b", "7a"],
    ["1 2", "12"],
    ["a 0.5", "a 0.50"],
    ["", "a"],
    ["0", "00"],
];
foreach ($pairs as [$a, $b]) {
    echo json_encode($a), ' ', json_encode($b), ' ', strnatcmp($a, $b), ' ', strnatcasecmp($a, $b), "\n";
}
$v = ["psr/log-implementation 3.0.0|2.0.0|1.0.0", "psr/log-implementation 1.0|2.0|3.0", "psr/log-implementation 1.0 || 2.0 || 3.0", "psr/log 1.0"];
sort($v, SORT_NATURAL);
print_r($v);
natcasesort($v);
print_r($v);
