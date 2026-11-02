<?php
// `$GLOBALS['x'] = &$v` rebinds the symbol-table entry itself (it must not
// write a reference *into* the old cell), including from a function and
// when the variable is bound to itself.
$i = 1;
$GLOBALS['i'] = &$i;
var_dump($i);

$y = 1;
$GLOBALS['x'] = &$y;
$y = 2;
var_dump($x);
$x = 3;
var_dump($y);
function setx() { global $x; $x = 4; }
setx();
var_dump($y);

$b = 1;
function alias() { $GLOBALS['a'] = &$GLOBALS['b']; }
alias();
$a = 5;
var_dump($b);

function local_ref() { $loc = 'local'; $GLOBALS['fromFn'] = &$loc; $loc = 'changed'; }
local_ref();
var_dump($fromFn);

// php creates a foreach's value variable before its key variable.
$arr = ['k' => 1];
foreach ($arr as $key => &$val) {
    $val = 2;
}
foreach ($arr as $k2 => $v2) {
}
var_dump(array_slice(array_keys($GLOBALS), -6));
