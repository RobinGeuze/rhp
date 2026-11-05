<?php
function popcount(int $n): int {
    $c = 0;
    while ($n) { $c += $n & 1; $n >>= 1; }
    return $c;
}
foreach ([0, 1, 7, 255, 1024, 0xFFFF] as $n) {
    echo "$n: " . popcount($n) . "\n";
}
echo (5 & 3), " ", (5 | 3), " ", (5 ^ 3), " ", (~5), "\n";
echo (1 << 10), " ", (1024 >> 3), "\n";
// flags
const READ = 1; const WRITE = 2; const EXEC = 4;
$perm = READ | EXEC;
echo ($perm & READ) ? "r" : "-";
echo ($perm & WRITE) ? "w" : "-";
echo ($perm & EXEC) ? "x" : "-";
echo "\n";
// swap without temp
$a = 5; $b = 9;
$a ^= $b; $b ^= $a; $a ^= $b;
echo "$a $b\n";
echo decbin(0b1010 | 0b0101), "\n";
echo PHP_INT_MAX & PHP_INT_MAX, "\n";
