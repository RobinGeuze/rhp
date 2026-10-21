<?php
$a = [1, 2, 3, 4, 5];
print_r(array_slice($a, 1, 3));
print_r(array_flip(["a" => 1, "b" => 2]));
var_dump(array_search(3, $a));
print_r(array_unique([1, 2, 2, 3, 3, 3]));
print_r(array_chunk([1, 2, 3, 4, 5], 2));
print_r(array_combine(["x", "y"], [10, 20]));
var_dump(array_is_list([1, 2, 3]), array_is_list([1 => "a"]));
echo array_key_first(["z" => 1, "y" => 2]), " ", array_key_last($a), "\n";
