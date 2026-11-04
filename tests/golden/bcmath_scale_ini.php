<?php
// bcmath's default scale is the bcmath.scale ini setting: bcscale() sets it,
// and ini_set() / ini_restore() change the scale the functions use.
set_error_handler(function ($errno, $msg) {
    echo "warning: $msg\n";
    return true;
});
var_dump(ini_get('bcmath.scale'), bcscale());
echo bcadd('1', '2.5'), "\n";
var_dump(bcscale(3));
var_dump(ini_get('bcmath.scale'));
echo bcadd('1', '2.5'), ' ', bcadd('1', '2.5', 1), "\n";
var_dump(ini_set('bcmath.scale', '5'));
var_dump(bcscale());
echo bcdiv('1', '3'), "\n";
// OnUpdateScale refuses a value outside 0..2147483647
var_dump(ini_set('bcmath.scale', '-1'));
var_dump(ini_set('bcmath.scale', '2147483648'));
var_dump(ini_get('bcmath.scale'), bcscale());
// and reads it as a quantity
var_dump(ini_set('bcmath.scale', 'abc'));
var_dump(ini_get('bcmath.scale'), bcscale());
var_dump(ini_set('bcmath.scale', '2abc'));
var_dump(ini_get('bcmath.scale'), bcscale());
var_dump(ini_set('bcmath.scale', ' 4 '));
var_dump(bcscale());
echo bcmul('1.25', '1.5'), "\n";
ini_restore('bcmath.scale');
var_dump(ini_get('bcmath.scale'), bcscale());
try {
    bcscale(-1);
} catch (ValueError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
var_dump(bcscale(null), bcscale(7), bcscale(0));
var_dump(ini_get_all('bcmath'));
echo bcpow('2', '-2'), ' ', bcsqrt('2'), "\n";
