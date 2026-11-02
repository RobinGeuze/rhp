<?php
// zend_ini_parse_quantity overflow: the "value is out of range" warning and
// the wrapped result php keeps (which memory_hard_limit_percent then accepts
// or refuses).
set_error_handler(function ($n, $m) {
    echo "  W: $m\n";
    return true;
});
foreach (["99999999999999999999", "9223372036854775808", "-9223372036854775809", "9007199254740992G",
          "0x8000000000000000", "-9007199254740993G", "9223372036854775807", "-9223372036854775808",
          "8589934591G", "-8589934592G", "0x7fffffffffffffff", "99999999999999999999k",
          "99999999999999999999kk", "99999999999999999999x", "0777777777777777777777777", "0200", "0x64",
          "0b1100100", "0o144", "-99999999999999999999", "100", "-1", "abc", "1e3"] as $v) {
    echo $v, "\n";
    var_dump(ini_set('memory_hard_limit_percent', $v));
    ini_restore('memory_hard_limit_percent');
}
