<?php
// curl argument handling: null to int/float parameters (one deprecation)
// and curl_share_init_persistent's option coercion.
set_error_handler(function ($no, $str) { echo "E[$no] $str\n"; return true; });

echo "== null arguments\n";
$h = curl_init();
$mh = curl_multi_init();
$sh = curl_share_init();
var_dump(is_string(curl_strerror(null)), is_string(curl_multi_strerror(null)), is_string(curl_share_strerror(null)));
var_dump(is_int(curl_pause($h, null)));
foreach ([
    fn () => curl_setopt($h, null, 1),
    fn () => curl_multi_setopt($mh, null, 1),
    fn () => curl_share_setopt($sh, null, 1),
] as $f) {
    try { var_dump($f()); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
}
var_dump(curl_multi_select($mh, null));
var_dump(is_int(call_user_func("curl_pause", $h, null)));

echo "== curl_share_init_persistent\n";
foreach ([[1.5], ['1x'], [NAN], [INF], ['abc'], [true], [null], [[]], ['2.5'], ['1e100'], [new stdClass]] as $a) {
    try {
        var_dump(curl_share_init_persistent($a));
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
