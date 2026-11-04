<?php
function fib(int $n, array &$memo = []): int {
    if ($n < 2) return $n;
    if (isset($memo[$n])) return $memo[$n];
    return $memo[$n] = fib($n - 1, $memo) + fib($n - 2, $memo);
}
for ($i = 0; $i <= 30; $i++) {
    echo fib($i), ($i % 10 === 9 ? "\n" : " ");
}
echo "\n";
// iterative DP
function coins(int $amount, array $coins): int {
    $dp = array_fill(0, $amount + 1, PHP_INT_MAX);
    $dp[0] = 0;
    foreach ($coins as $c) {
        for ($i = $c; $i <= $amount; $i++) {
            if ($dp[$i - $c] !== PHP_INT_MAX) {
                $dp[$i] = min($dp[$i], $dp[$i - $c] + 1);
            }
        }
    }
    return $dp[$amount];
}
echo coins(63, [1, 5, 10, 25]), "\n";
echo coins(11, [1, 2, 5]), "\n";
