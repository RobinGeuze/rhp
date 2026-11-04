<?php
function addTax(array &$prices, float $rate): void {
    foreach ($prices as &$p) $p *= (1 + $rate);
    unset($p);
}
$prices = [10.0, 20.0, 30.0];
addTax($prices, 0.1);
echo implode(",", array_map(fn($p) => round($p, 2), $prices)), "\n";

$a = 1;
$b = &$a;
$b = 5;
echo "$a $b\n";

$arr = [1, 2, 3];
$ref = &$arr[1];
$ref = 99;
print_r($arr);

function &getRef(array &$a, int $i) {
    return $a[$i];
}
$data = [10, 20, 30];
$r = &getRef($data, 0);
$r = 100;
echo $data[0], "\n";

$x = ['count' => 0];
$inc = function () use (&$x) { $x['count']++; };
$inc(); $inc();
echo $x['count'], "\n";

[$m, $n] = [1, 2];
[$m, $n] = [$n, $m];
echo "$m $n\n";
