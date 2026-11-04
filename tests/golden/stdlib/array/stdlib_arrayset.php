<?php
echo implode(",", array_diff_assoc(['a'=>'x','b'=>'y','c'=>'z'], ['a'=>'x','b'=>'q'])), "\n";
echo implode(",", array_intersect_assoc(['a'=>1,'b'=>2,'c'=>3], ['a'=>1,'c'=>3,'b'=>9])), "\n";
var_dump(array_find([1,2,3,4], fn($v) => $v > 2));
var_dump(array_find_key(['a'=>1,'b'=>2,'c'=>3], fn($v) => $v === 2));
var_dump(array_any([1,2,3], fn($v) => $v > 2));
var_dump(array_all([1,2,3], fn($v) => $v > 0));
var_dump(array_all([1,2,3], fn($v) => $v > 1));
var_dump(key_exists('b', ['a'=>1, 'b'=>2]));
$r = array_replace_recursive(['a'=>['x'=>1,'y'=>2]], ['a'=>['y'=>9,'z'=>3]]);
echo json_encode($r), "\n";
