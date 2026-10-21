<?php
$a = ["x" => 1, "y" => 2, "z" => 3];
print_r(array_keys($a));
print_r(array_values($a));
$m = array_merge([1, 2], [3, 4], ["k" => 5]);
print_r($m);
print_r(array_reverse([10, 20, 30]));
$r = range(1, 5);
echo array_sum($r), "\n";
print_r(range(3, 1));
foreach (array_keys($a) as $k) {
    echo "$k ";
}
echo "\n";
