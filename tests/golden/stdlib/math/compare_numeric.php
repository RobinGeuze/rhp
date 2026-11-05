<?php
set_error_handler(function ($no, $msg) { echo "[$no] $msg\n"; return true; });
// zend_compare corners: NAN against arrays, overflowing integer strings,
// -0.0 from strings and unary plus.
$nan = NAN;
var_dump($nan < [], $nan <= [1], $nan > [], $nan <=> [], [] <=> $nan, $nan == [], [1] > $nan);
var_dump("9223372036854775807" == "9223372036854775808", "9223372036854775808" <=> "9223372036854775809");
var_dump("1e1000" == "2e1000", "-1e1000" <=> "-2e1000", "9223372036854775808" > "1.0", "100" <=> "9223372036854775808");
var_dump(" 9223372036854775808" == "9223372036854775808 ", "0x10" == "16");
var_dump((float)"-0", +(-0.0), -"0", (float)" -0.0", floatval("-0abc"));
