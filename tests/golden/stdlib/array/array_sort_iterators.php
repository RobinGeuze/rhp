<?php
// Sorting during foreach by reference or array_walk over a table with
// holes: a packed table goes through zend_hash_rehash first (iterators move
// to the next element), a hash drops its holes without moving them.
$a = [1, 2, 3, 4, 5];
unset($a[1], $a[3]);
foreach ($a as $k => &$v) {
    echo "$k=$v ";
    if ($k === 2) {
        sort($a);
    }
}
unset($v);
echo "\n";

$a = [5, 1, 4, 2, 3];
unset($a[0]);
array_walk($a, function ($v, $k) use (&$a) {
    echo "$k=$v ";
    if ($k === 1) {
        usort($a, fn($x, $y) => $x <=> $y);
    }
});
echo json_encode($a), "\n";

$a = ['a' => 3, 'b' => 1, 'c' => 2, 'd' => 0];
unset($a['b']);
foreach ($a as $k => &$v) {
    echo "$k=$v ";
    if ($k === 'a') {
        asort($a);
    }
}
unset($v);
echo json_encode($a), "\n";

$a = ['a' => 3, 'b' => 1, 'c' => 2, 'd' => 0];
unset($a['b']);
array_walk($a, function ($v, $k) use (&$a) {
    echo "$k=$v ";
    if ($k === 'a') {
        uasort($a, fn($x, $y) => $x <=> $y);
    }
});
echo json_encode($a), "\n";
