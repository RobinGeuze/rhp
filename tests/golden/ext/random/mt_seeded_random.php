<?php
// The seeded Mt19937 engine behind mt_rand/rand/shuffle/array_rand/str_shuffle.
mt_srand(1);
echo mt_rand(), ' ', mt_rand(1, 100), ' ', rand(100, 1), ' ', mt_rand(0, PHP_INT_MAX), ' ', rand(), "\n";
mt_srand(42);
$seq = [];
for ($i = 0; $i < 8; $i++) { $seq[] = mt_rand(-5, 5); }
echo implode(',', $seq), "\n";
mt_srand(1);
$a = [1, 2, 3, 4];
shuffle($a);
echo implode(',', $a), "\n";
mt_srand(7);
$b = ['x' => 'a', 'y' => 'b', 'z' => 'c', 9 => 'd', 'e', 'f'];
shuffle($b);
var_dump($b);
mt_srand(5);
var_dump(array_rand(['a' => 1, 'b' => 2, 'c' => 3]), array_rand(range(0, 9), 3), array_rand(range(0, 9), 8));
mt_srand(3);
echo str_shuffle('abcdefgh'), "\n";
mt_srand(3, MT_RAND_MT19937);
echo mt_rand(1, 6), mt_rand(1, 6), mt_rand(1, 6), "\n";
foreach ([fn() => mt_rand(5, 1), fn() => array_rand([]), fn() => array_rand([1, 2], 3), fn() => array_rand([1], 0)] as $f) {
    try { $f(); } catch (ValueError $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}
$e = [];
var_dump(shuffle($e), $e, mt_getrandmax(), getrandmax());
