<?php
// Argument coercions mbstring.c gets from zpp and zend_operators, and the
// wrapping arithmetic of mb_strcut.
set_error_handler(function ($no, $msg) { echo "[$no] $msg\n"; return true; });
function t($f) {
    try { var_dump($f()); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}

echo "-- numeric entity maps (zval_try_get_long)\n";
foreach ([1e20, -1e19, INF, -INF, NAN, 0.5, "1e999", "-1e999", "9223372036854775808",
          "-9223372036854775809", "9.2233720368547758e18", "12abc", "1.5"] as $low) {
    $map = [$low, 0x10FFFF, 0, 0xFFFFFF];
    t(fn() => mb_encode_numericentity("a\u{e9}\u{20ac}", $map, "UTF-8"));
    t(fn() => mb_decode_numericentity("&#97;&#233;", $map, "UTF-8"));
}
t(fn() => mb_encode_numericentity("a", [0, 0x10FFFF, 1.5e10, 0xFFFFFF], "UTF-8"));

echo "-- mb_substitute_character (Z_PARAM_STR_OR_LONG)\n";
foreach ([true, false, 65.5, 66.0, INF, -INF, NAN, 1e30, 9.2233720368547758e18,
          -9.2233720368547758e18, "66", "1e3", "LONG"] as $v) {
    t(fn() => mb_substitute_character($v));
    t(fn() => mb_substitute_character());
}

echo "-- mb_strcut extremes\n";
t(fn() => mb_strcut("abc", PHP_INT_MAX, PHP_INT_MIN));
t(fn() => mb_strcut("abc", -2, PHP_INT_MIN));
t(fn() => mb_strcut("abc", PHP_INT_MIN, PHP_INT_MAX));
t(fn() => mb_strcut("abc", 1, -PHP_INT_MAX));
