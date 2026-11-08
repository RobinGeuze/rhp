<?php
$t = 0; $a = [1, 2, 3, 4];
for ($i = 0; $i < 1000000; $i++) {
    $f = function ($x) use ($i) { return $x + $i; };
    $g = fn($x) => $x * 2;
    $t = ($t + $f(1) + $g($i)) & 0xffff;
}
for ($i = 0; $i < 100000; $i++) { $t += array_sum(array_map(fn($v) => $v + $i, $a)) & 7; }
echo $t, "\n";
