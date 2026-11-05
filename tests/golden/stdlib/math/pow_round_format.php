<?php
set_error_handler(function ($no, $msg) { echo "[$no] $msg\n"; return true; });
// pow() and ** share pow_function: int powers by squaring, the overflow
// sign, 0 ** negative deprecated, operand errors.
var_dump(2 ** 62, 2 ** 63, (-2) ** 63, (-3) ** 41, 1 ** PHP_INT_MAX, 0 ** 5);
var_dump(pow(-2, 63), pow("3", "4"), pow(2.5, 2), pow(true, 3));
$z = 0;
var_dump($z ** -1, pow(0.0, -2), pow(0, -1.5));
$e = -1; $z **= $e; var_dump($z);
foreach ([[[], 1], [1, "x"], [new stdClass, 2]] as [$a, $b]) {
    try { var_dump(pow($a, $b)); } catch (Throwable $t) { echo get_class($t), ": ", $t->getMessage(), "\n"; }
}
var_dump(fpow(0, -1), fpow(-8, 1/3), fpow(2, 0.5));
// round() with RoundingMode and its int modes
foreach (RoundingMode::cases() as $m) {
    echo $m->name, ": ", round(2.5, 0, $m), " ", round(-2.5, 0, $m), " ", round(1.955, 2, $m), "\n";
}
var_dump(round(-0.4), round(5, -1), round(15, -1, PHP_ROUND_HALF_EVEN));
// number_format: ints keep every digit, negative decimals, separators
var_dump(number_format(PHP_INT_MAX), number_format(PHP_INT_MIN, 2), number_format(-1234567, -3));
var_dump(number_format(2**53 + 1.0), number_format(-0.4), number_format(1.005, 2), number_format(1e15, 2, ',', '.'));
var_dump(number_format(1234.5678, 3, '', ''), number_format(0.5), number_format(-0.5), number_format(5e-324, 330) === '0.' . str_repeat('0', 330));
var_dump(number_format(INF), number_format(-INF, 2), number_format(NAN));
// base conversion: whitespace and prefixes, invalid characters
var_dump(hexdec(" 0xff "), octdec("0o777"), bindec("0b1010"), bindec(" 1 0 1 "), base_convert("zz", 36, 2));
try { base_convert(str_repeat("z", 300), 36, 10); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
// intval with a base is C strtol (plus 0b for base 0 and 2)
var_dump(intval("0x1A", 16), intval("0x1A", 0), intval("012", 0), intval("0o17", 8), intval("  -0b101", 0));
var_dump(intval("99999999999999999999", 16), intval("-99999999999999999999", 36), intval("12", 1), intval("\v\f42", 8));
