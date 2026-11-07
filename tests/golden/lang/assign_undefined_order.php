<?php
// php's read order for undefined variables in assignments: a compound
// assignment reads its right operand first; an assignment's last key is
// read after the container was checked (false converted, a scalar
// refused, a string held).

set_error_handler(function ($no, $msg) {
    echo "handler: $msg\n";
    return true;
});

$a .= $b;
var_dump($a);
$n += $m;
var_dump($n);
$c = 1;
$c *= $d;
var_dump($c);

// The handler clobbers the target while the right operand warns.
set_error_handler(function ($no, $msg) {
    echo "clobber: $msg\n";
    $GLOBALS['x'] = 'set';
    return true;
});
$x .= $y;
var_dump($x);

set_error_handler(function ($no, $msg) {
    echo "handler: $msg\n";
    return true;
});
unset($x, $y);

$arr[$k1] = 1;
var_dump($arr);
$f = false;
$f[$k2] = 2;
var_dump($f);
$s = "abc";
$s[$k3] = "x";
var_dump($s);
$i = 5;
try {
    $i[$k4] = 1;
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
$nested = [0 => false];
$nested[0][$k5] = 3;
var_dump($nested);
$r = $q[$k6] = 4;
var_dump($r, $q);

// A handler that releases the false container: nothing is written.
set_error_handler(function ($no, $msg) {
    echo "release: $msg\n";
    $GLOBALS['g'] = 'gone';
    return true;
});
$g = [false];
$g[0][$k7] = 5;
var_dump($g);

// A handler that replaces a false container while the key warns: php's
// converted array is gone, so nothing is written.
set_error_handler(function ($no, $msg) {
    echo "replace: $msg\n";
    if (str_starts_with($msg, 'Undefined variable')) {
        $GLOBALS['h'] = 'zz';
    }
    return true;
});
$h = false;
$res = ($h[$k8] = 6);
var_dump($h, $res);
