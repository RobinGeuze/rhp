<?php
set_error_handler(function ($no, $msg) { echo "warning: $msg\n"; return true; });
// A by-reference foreach holds a reference to the array it iterates:
// rebinding the iterated variable (to the loop variable) leaves the loop
// on the original array (VarDumper's ReflectionCaster does this).
function f($c) {
    $a = [];
    if ($v = $c) {
        foreach ($v as $k => &$v) {
            $a[$k] = &$v;
        }
        unset($v);
    }
    var_dump($a);
}
f(['o' => new stdClass, 'z' => 2, 'n' => null]);

$v = ['x' => 1, 'y' => 2];
foreach ($v as $k => &$v) {
    echo $k, "\n";
}
var_dump($v);

// The key variable aliasing the source.
$k = [10, 20, 30];
foreach ($k as $k => &$val) {
    echo $k, " ", $val, "\n";
}
var_dump($k);

// Writes through the variable still reach the iterated array.
$w = [1, 2, 3];
foreach ($w as $i => &$e) {
    if ($i === 0) {
        $w[] = 4;
    }
    $e *= 10;
}
unset($e);
var_dump($w);

// Assigning a non-array to the variable ends the loop with a warning.
$s = [1, 2, 3];
foreach ($s as &$e) {
    echo $e, "\n";
    $s = 5;
}
var_dump($s);

// In the global scope the variable stays a global.
function g() { global $gl; return $gl; }
$gl = [1, 2];
foreach ($gl as &$e) { $e++; }
unset($e);
var_dump(g());
