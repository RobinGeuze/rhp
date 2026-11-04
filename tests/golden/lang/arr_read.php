<?php
$a = [10, 20, 30];
echo $a[0], " ", $a[1], " ", $a[2], "\n";
$m = ["x" => 1, "y" => 2];
echo $m["x"], " ", $m["y"], "\n";
$nested = [[1, 2], [3, 4]];
echo $nested[0][1], " ", $nested[1][0], "\n";
echo $a[99] ?? "missing", "\n";
