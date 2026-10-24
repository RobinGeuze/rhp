<?php
// nested loops with labels via flags
$found = null;
$grid = [[1, 2, 3], [4, 5, 6], [7, 8, 9]];
foreach ($grid as $i => $row) {
    foreach ($row as $j => $v) {
        if ($v === 5) { $found = [$i, $j]; break 2; }
    }
}
echo implode(",", $found), "\n";

// do-while
$n = 100; $steps = 0;
do {
    $n = ($n % 2 === 0) ? $n / 2 : 3 * $n + 1;
    $steps++;
} while ($n !== 1);
echo "collatz(100): $steps steps\n";

// switch fallthrough
function grade(int $score): string {
    switch (true) {
        case $score >= 90: return 'A';
        case $score >= 80: return 'B';
        case $score >= 70: return 'C';
        default: return 'F';
    }
}
echo grade(95), grade(85), grade(60), "\n";

// ternary chains and coalesce
$v = null;
echo $v ?: 'empty', "\n";
echo ($x = 5) > 3 ? 'big' : 'small', "\n";
$config = ['timeout' => 0];
echo $config['timeout'] ?: 30, "\n";
echo $config['timeout'] ?? 30, "\n";

// match with no arg match
$code = 404;
echo match (true) {
    $code < 300 => 'ok',
    $code < 400 => 'redirect',
    $code < 500 => 'client error',
    default => 'server error',
}, "\n";
