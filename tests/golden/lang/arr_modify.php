<?php
$a = [10, 20, 30];
$a[0] += 5;
$a[1] *= 2;
$a[2]++;
var_dump($a);
$counts = [];
$counts["x"] = 0;
$counts["x"]++;
$counts["x"]++;
echo $counts["x"], "\n";
