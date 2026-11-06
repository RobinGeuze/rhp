<?php
// The internal pointer stays on its element when others are removed.
$a = [1, 2, 3, 4];
next($a); next($a);
unset($a[0]);
var_dump(key($a), current($a));
unset($a[2]);
var_dump(key($a));
// Past the end: a later append becomes current.
end($a); next($a);
var_dump(key($a));
$a[] = 5;
var_dump(key($a), current($a));
// prev() from the first element also goes past the end.
reset($a); prev($a);
var_dump(key($a));
$a[] = 6;
var_dump(current($a));
next($a); next($a); next($a);
var_dump(key($a));
// A copy-on-write separation restarts a pointer past the end at 0
// (zend_array_dup).
$e = [1, 2];
end($e); next($e);
$copy = $e;
$e[] = 3;
var_dump(key($e));

// The next free key.
$b = [-3 => 'x'];
array_pop($b);
$b[] = 'y';
var_dump($b);
$c = [5 => 'a', 'k' => 'b', 9 => 'c'];
array_shift($c);
$c[] = 'd';
var_dump($c);
try {
    $d = [PHP_INT_MAX => 1];
    $d[] = 2;
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
unset($d[PHP_INT_MAX]);
$d[] = 3;
var_dump(array_keys($d) === [PHP_INT_MAX]);

// array_walk sees the live array.
$w = [1, 2, 3];
array_walk($w, function ($v, $k) use (&$w) {
    echo "$k=$v\n";
    if ($k === 0) {
        $w[] = 4;
        unset($w[1]);
    }
});
$w = ['a' => 1, 'b' => 2];
array_walk($w, function (&$v, $k) use (&$w) {
    $copy = $w;
    $w['c'] = 3;
    $v *= 10;
    echo count($copy), " $k\n";
});
var_dump($w);
// The third argument only when given.
array_walk($w, function () { echo func_num_args(), " "; });
array_walk($w, function () { echo func_num_args(), " "; }, null);
echo "\n";
// Objects: the property table by mangled name.
class P { public $a = 1; protected $b = 2; private $c = 3; }
$p = new P;
array_walk($p, function (&$v, $k) { $v = $v * 100; echo strlen($k), " "; });
echo "\n";
var_dump($p);

// Sorts keep the references they hold.
$x = 3; $y = 1;
$s = [&$x, 2, &$y];
sort($s);
$y = 100;
var_dump($s);
$t = ['b' => 'B', 'a' => 'a', 'C' => 'c'];
ksort($t, SORT_STRING | SORT_FLAG_CASE);
var_dump(array_keys($t));
$u = [10, '9', 'abc', 1.5, true, null];
rsort($u);
var_dump($u);
var_dump(array_unique([3, '3', 1, '01', 'a', 'A'], SORT_REGULAR));
// A sort with nothing to sort leaves the table alone.
$z = [5];
unset($z[0]);
sort($z);
$z[] = 1;
var_dump(array_keys($z));
$z = [4 => 'x'];
end($z); next($z);
uasort($z, fn($p, $q) => 0);
var_dump(key($z));
