<?php
function primes(int $n): array {
    $sieve = array_fill(0, $n + 1, true);
    $sieve[0] = $sieve[1] = false;
    for ($i = 2; $i * $i <= $n; $i++) {
        if ($sieve[$i]) {
            for ($j = $i * $i; $j <= $n; $j += $i) $sieve[$j] = false;
        }
    }
    return array_keys(array_filter($sieve));
}
$p = primes(50);
echo implode(" ", $p), "\n";
echo "count: " . count($p) . "\n";
echo "sum: " . array_sum($p) . "\n";

function isPrime(int $n): bool {
    if ($n < 2) return false;
    for ($i = 2; $i <= (int)sqrt($n); $i++) {
        if ($n % $i === 0) return false;
    }
    return true;
}
$twins = [];
foreach ($p as $x) {
    if (isPrime($x + 2) && in_array($x + 2, $p)) $twins[] = "($x,".($x+2).")";
}
echo implode(" ", $twins), "\n";
