<?php
// Comparisons, type juggling and casts.

// comparison_operators_at_boundaries
var_dump(5 > 5, 5 >= 5, 5 < 5, 5 <= 5, 5 == 5, 5 != 5, 5 <> 5, 4 < 5, 6 > 5);

// spaceship_operator
echo 1 <=> 2, 2 <=> 2, 3 <=> 2, "\n";
var_dump([1, 2] <=> [1, 3]);
echo ('a' <=> 'b'), ('b' <=> 'a'), ('a' <=> 'a'), "\n";

// strict_vs_loose_equality
var_dump(1 === 1.0, 1 == 1.0, '1' === 1, '1' == 1);

// php8_numeric_string_juggling: a leading-numeric string is not numeric
var_dump('10abc' == 10);

// null_juggling
var_dump(null == 0, null == '', null === null, null == 'x');

// numeric_string_arithmetic
var_dump('10' + 5, '3.5' + '1.5');

// string_comparison_lexicographic
echo strcmp('a', 'b'), ' ', strcmp('b', 'a'), ' ', strcmp('a', 'a'), "\n";

// int_casts / float_casts
var_dump((int)3.9, (int)true, (float)5);
