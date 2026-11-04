<?php
// php reads these settings and ini operands with C atoll/atoi: a number
// past the long range saturates, and atoi then cuts it to 32 bits.
var_dump(parse_ini_string(
    "a = 99999999999999999999 | 0\n" .
    "b = -99999999999999999999 | 0\n" .
    "c = 4294967297 | 0\n" .
    "d = 2147483648 | 0\n" .
    "e = 281474976710657 | 0\n" .
    "f = -9223372036854775808 | 0\n"
));
foreach (['99999999999999999999', '-99999999999999999999', '4294967298', ' 8 ', '8x'] as $v) {
    ini_set('error_reporting', $v);
    var_dump(error_reporting());
}
