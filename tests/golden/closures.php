<?php
$sq = fn($x) => $x * $x;
echo $sq(6), "\n";

$base = 100;
$add = fn($x) => $x + $base;
echo $add(23), "\n";

$mul = function($x) use ($base) { return $x * $base; };
echo $mul(3), "\n";

$nums = [1, 2, 3, 4, 5];
echo implode(",", array_map(fn($n) => $n * 2, $nums)), "\n";
echo implode(",", array_filter($nums, fn($n) => $n % 2 === 1)), "\n";
echo array_reduce($nums, fn($c, $n) => $c + $n, 0), "\n";

$compose = fn($f, $g) => fn($x) => $f($g($x));
$h = $compose(fn($x) => $x + 1, fn($x) => $x * 3);
echo $h(10), "\n";

echo (fn($a, $b) => $a . "-" . $b)("x", "y"), "\n";
echo (function() { return 42; })(), "\n";

$ops = ['add' => fn($a, $b) => $a + $b, 'sub' => fn($a, $b) => $a - $b];
echo $ops['add'](10, 5), " ", $ops['sub'](10, 5), "\n";

echo call_user_func($sq, 9), "\n";
echo call_user_func_array(fn($a, $b, $c) => $a + $b + $c, [1, 2, 3]), "\n";

$c = 5;
$f = function() use ($c) { $c++; return $c; };
echo $f(), " ", $c, "\n";

var_dump(is_callable($sq));
echo gettype($sq), " ", get_debug_type($sq), "\n";
