<?php
$a = ['a' => 'green', 'b' => 'brown', 'c' => 'blue', 'red', 10 => '1'];
$b = ['a' => 'GREEN', 'B' => 'brown', 'yellow', 'red', '10' => 1];
$c = ['A' => 'green', 'c' => 'blue', 0 => 'red'];

$ci = fn($x, $y) => strcasecmp((string) $x, (string) $y);
$cmp = fn($x, $y) => $x <=> $y;

print_r(array_diff_uassoc($a, $b, $ci));
print_r(array_diff_uassoc($a, $b, $c, $ci));
print_r(array_udiff_assoc($a, $b, $ci));
print_r(array_udiff_assoc($a, $b, $c, $cmp));
print_r(array_udiff_uassoc($a, $b, $ci, $ci));
print_r(array_intersect_ukey($a, $b, $ci));
print_r(array_intersect_ukey($a, $b, $c, $ci));
print_r(array_intersect_uassoc($a, $b, $ci));
print_r(array_uintersect_assoc($a, $b, $ci));
print_r(array_uintersect_uassoc($a, $b, $ci, $ci));
print_r(array_uintersect_uassoc($a, $b, $c, $ci, $ci));
print_r(array_intersect_ukey(['PATH' => 1, 'Home' => 2, 'x' => 3], ['path' => 0, 'HOME' => 0], 'strcasecmp'));
print_r(array_intersect_ukey([], ['a' => 1], 'strcasecmp'));
print_r(array_diff_uassoc(['a' => 1], [], 'strcmp'));
// existing family members still behave
print_r(array_diff_ukey($a, $b, $ci));
print_r(array_udiff($a, $b, $ci));
print_r(array_uintersect($a, $b, $ci));
try {
    array_intersect_ukey($a, 'nope', $ci);
} catch (TypeError $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
