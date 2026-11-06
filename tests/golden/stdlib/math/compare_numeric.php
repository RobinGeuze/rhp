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
// 20+ integer digits set oflow even with a fraction or exponent.
var_dump("18446744073709551616" == "18446744073709551616.0", "18446744073709551616.5" <=> "18446744073709551616",
    "000000000000000000001.5" == "1.5", "-18446744073709551616e0" == "-18446744073709551616", "99999999999999999999.0" <=> "99999999999999999999.0");
