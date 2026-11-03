<?php
// inet_* go through libc as in php. Only forms glibc and macOS agree on are
// here; the rest is compared against the live php (rhp-difftest tests/inet.rs).
set_error_handler(function ($no, $str) {
    echo "warning: $str\n";
    return true;
});
foreach (['1.2.3.4', '1.2.3', '::1', '::ffff:1.2.3.4', '::1.2.3.4', '1:0:0:1:0:0:0:1',
    '2001:db8::', '0:0:0:0:0:0:0:0', '1.2.3.4.5', '255.255.255.256', ' 1.2.3.4', '0x1.2.3.4',
    'abc', ''] as $s) {
    $p = inet_pton($s);
    echo var_export($s, true), ' ', $p === false ? 'false' : bin2hex($p), ' ',
        $p === false ? '-' : inet_ntop($p), ' ', var_export(ip2long($s), true), "\n";
}
foreach (['00000000000000000000ffff01020304', '00000000000000000000000001020304',
    '0000000000000000000000000000000a', '20010db8000000000000000000000001', '7f000001'] as $h) {
    echo $h, ' ', inet_ntop(hex2bin($h)), "\n";
}
var_dump(inet_ntop('abc'), long2ip(-1), long2ip(4294967296 + 5));
foreach (['ip2long', 'inet_pton', 'gethostbyaddr'] as $f) {
    try {
        $f("1.2.3.4\0x");
    } catch (ValueError $e) {
        echo $e->getMessage(), "\n";
    }
}
var_dump(gethostbyaddr('1.2.3'), gethostbyaddr(''));
