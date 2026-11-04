<?php
// References through aliases, destructuring and spread.

// reference_parameter_writes_through: an argument that is already an alias
$a = 5; $b = &$a;
function inc(&$x) { $x++; }
inc($b);
echo $a, ' ', $b, "\n";

// list_destructuring_variants: skipped slots
[, $q, , $s] = [1, 2, 3, 4];
echo "$q$s\n";

// destructuring_swap
$l = 1; $r = 2;
[$l, $r] = [$r, $l];
echo "$l $r\n";

// array_spread_numeric_and_string_keys: string keys keep their order
$m = ['x' => 1, 'y' => 2];
$n = [...$m, 'z' => 3];
echo implode(',', array_keys($n)), "\n";
