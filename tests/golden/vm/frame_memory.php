<?php
// Call frames count toward memory_get_usage() as php's VM stack pages
// (256 KiB each): deep recursion takes pages and gives them back, and
// there is no fixed depth limit.
function r($n) {
    if ($n == 0) {
        return memory_get_usage();
    }
    return r($n - 1);
}
function sum($n) { return $n == 0 ? 0 : $n + sum($n - 1); }
class Deep {
    function down(int $n): int { return $n == 0 ? 0 : 1 + $this->down($n - 1); }
}
// Warm up (first calls cache things of their own); nothing is printed
// until the end (output takes memory too).
r(20000);
$shallow = $deep = $after = $total = $after_sum = $methods = 0;
$base = memory_get_usage();
$shallow = r(100) - $base;
$deep = r(20000) - $base;
$after = memory_get_usage() - $base;
$total = sum(100000);
$after_sum = memory_get_usage() - $base;
$methods = (new Deep)->down(70000);
echo "shallow: ", $shallow < 4096 ? "same page" : "grew $shallow", "\n";
echo "deep: ", $deep > 2000000 && $deep < 4000000 ? "pages" : "unexpected $deep", "\n";
echo "after: $after\n";
echo "sum: $total\n";
echo "after sum: $after_sum\n";
echo "methods: $methods\n";
