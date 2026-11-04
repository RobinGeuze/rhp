<?php
function increment(&$n) {
    $n++;
}
$x = 5;
increment($x);
increment($x);
echo $x, "\n";
function swap(&$a, &$b) {
    $tmp = $a;
    $a = $b;
    $b = $tmp;
}
$p = 1;
$q = 2;
swap($p, $q);
echo "$p $q\n";
function addTo(&$arr, $val) {
    $arr[] = $val;
}
$list = [1, 2];
addTo($list, 3);
addTo($list, 4);
print_r($list);
