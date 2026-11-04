<?php
// Strings: interpolation, quoting, search predicates, padding, printf.

// double_quote_interpolation
$a = [1, 2];
echo "x={$a[1]} y=$a[0]\n";

// single_quotes_are_literal
$n = 'World';
echo 'Hello $n', "\n";

// string_search_predicates
var_dump(str_contains('hello', 'ell'), str_starts_with('hello', 'he'), str_ends_with('hello', 'lo'),
    str_contains('hello', 'zzz'));

// str_pad_variants
echo str_pad('5', 3, '0', STR_PAD_LEFT), '|', str_pad('5', 3), "|\n";

// printf_and_sprintf_format_specs
printf('%d %05.2f %s %x', 42, 3.14159, 'hi', 255);
echo "\n", sprintf("[%'*10s]", 'r'), ' ', sprintf('%b %o', 5, 8), "\n";
