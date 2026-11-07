<?php
// array_splice and array_walk over tables with holes: HashTable iterators
// sit on positions (holes included) and array_walk's moves to the next
// element, as php_splice and php_array_walk expect.
$a = [1, 2, 3, 4, 5, 6];
unset($a[1]);
array_walk($a, function ($v, $k) use (&$a) {
    echo "$k=$v ";
    if ($k === 2) {
        array_splice($a, 0, 2, ['x', 'y', 'z']);
    }
});
echo "\n";

$a = [1, 2, 3, 4, 5, 6];
unset($a[1]);
foreach ($a as $k => &$v) {
    echo "$k=$v ";
    if ($k === 3) {
        array_splice($a, 1, 1);
    }
}
unset($v);
echo "\n";

$a = ['k0' => 0, 'k1' => 1, 'k2' => 2, 'k3' => 3, 'k4' => 4, 'k5' => 5];
unset($a['k0'], $a['k4']);
array_walk($a, function ($v, $k) use (&$a) {
    echo "$k ";
    if ($k === 'k3') {
        echo count(array_splice($a, 8, 3)), "! ";
    }
});
echo "\n";

$a = [1 => 1, 3 => 3];
array_walk($a, function ($v, $k) use (&$a) {
    echo "$k ";
    if ($k === 1) {
        array_splice($a, 8, 3, ['r']);
    }
});
echo json_encode($a), "\n";

$a = ['a' => 1, 'b' => 2, 'c' => 3, 'd' => 4];
array_walk($a, function ($v, $k) use (&$a) {
    echo "$k ";
    if ($k === 'a') {
        unset($a['b']);
        $a['e'] = 5;
    }
});
echo "\n";
