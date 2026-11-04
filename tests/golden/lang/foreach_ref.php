<?php
$a = [1, 2, 3, 4];
foreach ($a as &$v) { $v *= 10; }
unset($v);
echo implode(",", $a), "\n";

$m = ['x' => 1, 'y' => 2, 'z' => 3];
foreach ($m as $k => &$v) { $v = "$k:$v"; }
unset($v);
echo implode(",", $m), "\n";

$grid = [[1, 2], [3, 4]];
foreach ($grid as &$row) {
    foreach ($row as &$cell) { $cell++; }
    unset($cell);
}
unset($row);
echo implode(",", array_map(fn($r) => implode("-", $r), $grid)), "\n";

// last-element reference gotcha
$arr = [1, 2, 3];
foreach ($arr as &$v) {}
foreach ($arr as $v) {}
echo implode(",", $arr), "\n";

// conditional modify
$nums = [5, 10, 15, 20];
foreach ($nums as &$n) { if ($n % 10 === 0) $n = -$n; }
unset($n);
echo implode(",", $nums), "\n";
