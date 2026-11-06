<?php
// array_walk keeps going over an array the callback sorts or splices:
// sorts keep the walk's position, array_splice moves it as php_splice does.
function w($a, $f) {
    array_walk($a, function (&$v, $k) use (&$a, $f) {
        echo "$k=$v ";
        $f($a, $k);
    });
    echo "| ", json_encode($a), "\n";
}
w([1, 2, 3, 4, 5], function (&$a, $k) { if ($k === 1) array_splice($a, 1, 2, ['x', 'y', 'z']); });
w([1, 2, 3, 4, 5], function (&$a, $k) { if ($k === 1) array_splice($a, 2, 2); });
w([1, 2, 3, 4, 5], function (&$a, $k) { if ($k === 1) $r = array_splice($a, 2, 2, 'q'); });
w([1, 2, 3, 4, 5], function (&$a, $k) { if ($k === 3) array_splice($a, 0, 1, ['a', 'b', 'c']); });
w([1, 2, 3, 4, 5], function (&$a, $k) { if ($k === 0) array_splice($a, 1); });
w([7], function (&$a, $k) { static $d = 0; if (!$d++) $r = array_splice($a, 0, null, ['r0', 'r1']); });
w(['a' => 1, 'b' => 2, 'c' => 3, 'd' => 4], function (&$a, $k) { if ($k === 'a') array_splice($a, 1, 2); });
w(['a' => 1, 'b' => 2, 'c' => 3, 'd' => 4], function (&$a, $k) { if ($k === 'b') array_splice($a, 0, 1, [9, 8]); });
w([3, 1, 2], function (&$a, $k) { if ($k === 0) sort($a); });
w(['b' => 2, 'a' => 1, 'c' => 3], function (&$a, $k) { static $d = 0; if (!$d++) ksort($a); });
w([5, 3, 4], function (&$a, $k) { static $d = 0; if ($d++ < 3) uasort($a, fn($x, $y) => $x <=> $y); });
w([3, 1, 2], function (&$a, $k) { if ($k === 0) array_multisort($a); });
mt_srand(3);
w([1, 2, 3, 4, 5, 6], function (&$a, $k) { if ($k === 1) shuffle($a); });
// Recursion is caught on the arrays recursed into, not the one the walk
// started from.
$r = [1, [2, 3]];
$r[1][] = &$r;
try {
    array_walk_recursive($r, function ($v) { echo "$v "; });
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
