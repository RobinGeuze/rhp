//! inet_pton / inet_ntop / ip2long / long2ip / gethostbyaddr against php. php
//! calls libc for these, so the accepted forms differ per platform (macOS
//! takes leading zeros and `%scope`, glibc doesn't); rhp must match the php on
//! the same machine. Skips without `php`.

use rhp_difftest::{diff_source, php_available};

const SRC: &str = r#"<?php
$in = ['1.2.3.4', '01.2.3.4', '1.2.3.04', '1.2.3', '::1', '::ffff:1.2.3.4', '::1.2.3.4',
    '1:0:0:1:0:0:0:1', '1:0:0:0:1:0:0:1', 'fe80::1%lo0', 'fe80::1%1', '2001:db8::',
    '0:0:0:0:0:0:0:0', '::ffff:0:0', '1::2:3:4:5:6:7', '1.2.3.4.5', '255.255.255.256',
    '::FFFF:abcd:ef01', ' 1.2.3.4', '1.2.3.4 ', '0x1.2.3.4', '1..2.3', '::1.2.3', 'abc', '',
    '1:2:3:4:5:6:7:8:9', '::ffff:1.2.3.4.5', '[::1]', '0.0.0.0', '255.255.255.255'];
foreach ($in as $s) {
    $p = inet_pton($s);
    echo var_export($s, true), ' ', $p === false ? 'false' : bin2hex($p), ' ',
        $p === false ? '-' : var_export(inet_ntop($p), true), ' ', var_export(ip2long($s), true), "\n";
}
foreach (['00000000000000000000ffff01020304', '00000000000000000000000001020304',
    '00010000000100000000000000000001', '0000000000000000000000000000000a',
    '20010db8000000000000000000000001', '00000000000000000000000000000001',
    'fe800001000000000000000000000001', '0000000000000000ffff000001020304', '7f000001', 'ab'] as $h) {
    echo $h, ' ', var_export(inet_ntop(hex2bin($h)), true), "\n";
}
foreach ([0, 1, -1, 16909060, 4294967295, 4294967296, PHP_INT_MAX, PHP_INT_MIN] as $n) {
    echo long2ip($n), "\n";
}
var_dump(gethostbyaddr('127.0.0.1'), @gethostbyaddr('1.2.3'), @gethostbyaddr('nope'));
"#;

#[test]
fn inet_functions_match_php() {
    if !php_available() {
        eprintln!("[inet] no php — skipping");
        return;
    }
    if let Some(why) = diff_source(SRC.as_bytes()).failure() {
        panic!("[inet] {why}");
    }
}
