<?php
// Arithmetic: precedence, integer division and modulo signs, float
// promotion, compound assignment, float echo precision.

// int_arithmetic
echo 2 + 3 * 4 - 1, ' ', (2 + 3) * 4, ' ', 7 - 10, ' ', 2 * -3, "\n";

// integer_division_truncates_toward_zero
echo intdiv(7, 2), ' ', intdiv(-7, 2), ' ', intdiv(7, -2), ' ', intdiv(-7, -2), "\n";

// modulo_sign_follows_dividend
echo -7 % 3, ' ', 7 % -3, ' ', -10 % 3, ' ', 10 % -3, ' ', -10 % -3, ' ', 10 % 3, "\n";

// fmod_is_float_remainder
echo fmod(7.5, 2), "\n";

// division_promotes_to_float_only_when_not_exact
var_dump(10 / 3, 10 / 2, 6 / 3, 10 / 4);

// int_overflow_promotes_to_float (a literal operand)
var_dump(9223372036854775807 + 1);

// mixed_int_float_arithmetic_types
var_dump(5 + 2.0, 5 * 2, 2.5 + 2.5);

// power_operator: right-associative
echo 2 ** 3 ** 2, "\n";

// increment_decrement: a numeric string increments as a number
$n = '9';
$n++;
var_dump($n);

// compound_assignment
$x = 1; $x += 2; $x *= 3; $x -= 1; echo $x, ' ';
$x = 10; $x /= 4; echo $x, ' ';
$x = 10; $x %= 3; echo $x, ' ';
$x = 2; $x **= 3; echo $x, ' ';
$x = 1; $x <<= 3; echo $x, "\n";

// chained_assignment_is_right_associative
$a = $b = $c = 5;
echo "$a$b$c\n";

// float_echo_uses_precision_14
echo 1/3, ' ', 100.0, ' ', 0.5, ' ', 1e10, ' ', 3.14, "\n";
