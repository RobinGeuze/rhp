<?php
// The source of a destructuring assignment is read before any target is written.
$a = [1, 2];
[$a, $b] = $a;
var_dump($a, $b);

$o = new stdClass;
$o->x = 5;
$a = [$o, 7];
[$a, $b] = $a;
var_dump($a->x, $b);

$a = ['k' => [3, 4], 'j' => 9];
['k' => [$a, $c], 'j' => $d] = $a;
var_dump($a, $c, $d);

// The expression's value is the old array.
$a = [1, 2];
$r = [$a, $b] = $a;
var_dump($r);

// A non-variable source is unaffected.
$a = [[5, 6]];
[$a, $b] = $a[0];
var_dump($a, $b);

function f() {
    $a = ['x', 'y'];
    $c = &$a;
    [$c, $b] = $a;
    var_dump($a, $b);
}
f();
