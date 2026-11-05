<?php
echo sqrt(2), "\n";
echo pow(2, 10), " ", 2 ** 0.5, "\n";
echo abs(-5), " ", abs(-5.5), "\n";
echo ceil(4.1), " ", floor(4.9), " ", round(4.5), " ", round(3.14159, 2), "\n";
echo max(1, 2, 3), " ", min([4, 2, 8]), "\n";
echo intdiv(17, 5), " ", 17 % 5, " ", fmod(17.5, 5), "\n";
echo gmp_or_not(), "\n";
function gmp_or_not() { return gcd(48, 36); }
function gcd(int $a, int $b): int { return $b === 0 ? $a : gcd($b, $a % $b); }
echo PHP_INT_MAX, "\n";
echo PHP_INT_MAX + 1, "\n";
echo 0.1 + 0.2, "\n";
echo 0.1 + 0.2 === 0.3 ? "eq" : "neq", "\n";
printf("%.17f\n", 0.1 + 0.2);
echo dechex(255), " ", hexdec("ff"), " ", decbin(10), " ", bindec("1010"), "\n";
echo base_convert("ff", 16, 2), "\n";
echo log(M_E), " ", log10(1000), " ", log(8, 2), "\n";
echo intval("0x1A", 16), " ", intval("42abc"), "\n";
var_dump(is_nan(sqrt(-1)));
try {
    var_dump(is_infinite(1 / 0.0 ?? INF));
} catch (DivisionByZeroError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
echo 7 <=> 3, " ", "a" <=> "b", "\n";
