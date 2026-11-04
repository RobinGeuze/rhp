<?php
// curl_setopt reads a long option with zval_get_long: a float that is no
// int warns (8.5), an object warns.
set_error_handler(function ($no, $msg) { echo "Warning: $msg\n"; return true; });
$ch = curl_init();
foreach ([1e20, NAN, "12abc", 3.7, true, new stdClass] as $v) {
    var_dump(curl_setopt($ch, CURLOPT_TIMEOUT, $v));
}
