<?php
// Arrays: copy-on-write, by-value passing, keys, and their builtins.

// array_copy_on_write_is_value_semantics
$a = [1, 2, 3]; $b = $a; $b[0] = 99; echo $a[0], ' ', $b[0], "\n";
$a = [[1, 2], [3, 4]]; $b = $a; $b[0][0] = 99; echo $a[0][0], ' ', $b[0][0], "\n";

// array_passed_by_value_to_function
function m($x) { $x[0] = 7; return $x; }
$c = [1]; $d = m($c); echo $c[0], ' ', $d[0], "\n";

// negative_and_sparse_keys_do_not_autorebase
$a = [1, 2, 3]; $a[-1] = 9; echo $a[-1], ' ', count($a), "\n";

// array_merge_string_keys_override_numeric_reindex
print_r(array_merge(['a' => 1], ['a' => 2, 'b' => 3]));

// array_slice_positive_and_negative
echo implode(',', array_slice([1, 2, 3, 4, 5], 1, 2)), ' ', implode(',', array_slice([1, 2, 3, 4, 5], -2)), "\n";

// array_key_exists_vs_isset: a key holding null
$a = ['x' => null];
var_dump(isset($a['x']), array_key_exists('x', $a), array_key_exists('y', $a));

// var_export_shapes
var_export([1, 'two', 3.5, true, null]);
echo "\n";
