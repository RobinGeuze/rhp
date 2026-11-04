<?php
function toRoman(int $n): string {
    $map = [1000 => 'M', 900 => 'CM', 500 => 'D', 400 => 'CD', 100 => 'C',
            90 => 'XC', 50 => 'L', 40 => 'XL', 10 => 'X', 9 => 'IX',
            5 => 'V', 4 => 'IV', 1 => 'I'];
    $out = '';
    foreach ($map as $val => $sym) {
        while ($n >= $val) { $out .= $sym; $n -= $val; }
    }
    return $out;
}
function fromRoman(string $s): int {
    $vals = ['I' => 1, 'V' => 5, 'X' => 10, 'L' => 50, 'C' => 100, 'D' => 500, 'M' => 1000];
    $total = 0; $prev = 0;
    for ($i = strlen($s) - 1; $i >= 0; $i--) {
        $cur = $vals[$s[$i]];
        $total += ($cur < $prev) ? -$cur : $cur;
        $prev = $cur;
    }
    return $total;
}
foreach ([4, 9, 14, 40, 90, 400, 1994, 2024, 3888] as $n) {
    $r = toRoman($n);
    echo "$n = $r = " . fromRoman($r) . "\n";
}
