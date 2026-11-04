<?php
function square($x) { return $x * $x; }
function isPositive($x) { return $x > 0; }
function addUp($carry, $x) { return $carry + $x; }
$nums = [1, -2, 3, -4, 5];
print_r(array_map("square", $nums));
print_r(array_map("strtoupper", ["a", "b"]));
print_r(array_filter($nums, "isPositive"));
echo array_reduce($nums, "addUp", 0), "\n";
print_r(array_map("square", [1, 2], [10, 20]) ?: array_map(null, [1, 2], [10, 20]));
echo call_user_func("strlen", "hello"), "\n";
echo call_user_func_array("max", [3, 7, 2]), "\n";
class Calc {
    public static function dbl($x) { return $x * 2; }
    public function trp($x) { return $x * 3; }
}
print_r(array_map(["Calc", "dbl"], [1, 2, 3]));
$c = new Calc();
print_r(array_map([$c, "trp"], [1, 2, 3]));
print_r(array_filter(["x" => 1, "y" => 0, "z" => 2]));
