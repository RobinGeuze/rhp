<?php
// How engine errors on a non-object or non-array name the value: a method
// call, a property fetch for write or reference, and an array offset read
// use zend_zval_value_name (true/false), list() zend_zval_type_name (bool).
set_error_handler(function ($no, $msg) {
    echo "Warning: $msg\n";
    return true;
});
foreach ([true, false, null, 1, 1.5, "s", STDIN] as $v) {
    echo "-- ", var_export($v, true), "\n";
    try { $v->foo(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
    try { $m = "bar"; $v->$m(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
    try { $v->spread(...[1]); } catch (Error $e) { echo $e->getMessage(), "\n"; }
    try { call_user_func_array([$v, "baz"], []); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    [$a, $b] = $v;
    $x = $v[0];
    try { $w = $v; $r = &$w->p; } catch (Error $e) { echo $e->getMessage(), "\n"; }
    try { $w = $v; $r = &$w->p->q; } catch (Error $e) { echo $e->getMessage(), "\n"; }
    try { $w = $v; $w->p = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
    try { $w = $v; $w->p[] = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
    $y = $v->p;
}
