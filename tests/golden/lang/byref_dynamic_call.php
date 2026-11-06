<?php
// A callable value's by-reference parameters (SEND_FUNC_ARG).
$f = function (&$r) { $r = 5; };
$a = [];
$f($a['x']);
var_dump($a);
$o = new stdClass;
$f($o->p);
var_dump($o);
$g = function ($v) { return $v; };
var_dump($g($a['x']));
$h = function (&$r) { $r[] = 1; };
$q = [];
$h($q['n']);
$h($q['n']);
var_dump($q);
try {
    $f([0, 1][0]);
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
try {
    $f($a[]);
    var_dump($a);
    $g($a[]);
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
