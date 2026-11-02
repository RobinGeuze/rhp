<?php
var_dump(array_first([3 => 'a', 1 => 'b']), array_last([3 => 'a', 1 => 'b']), array_first([]), array_last([]));
var_dump(array_first(['k' => ['x']]));
foreach (['Az', 'zz', 'a9', 'Zz', '9', 'b', 'A99', 'zZ9'] as $s) {
    var_dump(str_increment($s));
}
foreach (['Ba', 'a10', 'Aa', '1', '10', 'zA', 'b', 'B00'] as $s) {
    var_dump(str_decrement($s));
}
foreach (['', 'a-b', 'é'] as $s) {
    try {
        str_increment($s);
    } catch (ValueError $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
foreach (['a', 'A', '0', '0a', '00'] as $s) {
    try {
        var_dump(str_decrement($s));
    } catch (ValueError $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
foreach (['192.168.1.1', '0.0.0.0', '255.255.255.255', '192.168.01.1', '1.2.3', '1.2.3.4.5',
          '256.1.1.1', '1..3.4', '+1.2.3.4', '1.2.3.4 ', '', 'a.b.c.d'] as $ip) {
    var_dump(ip2long($ip));
}
foreach ([0, -1, 3232235777, 4294967296, 16909060, PHP_INT_MAX] as $n) {
    var_dump(long2ip($n));
}
var_dump(zend_version() === '4.' . PHP_MINOR_VERSION . '.' . PHP_RELEASE_VERSION, get_cfg_var('nope'));
var_dump(cli_set_process_title('rhp-test'));
var_dump(array_keys(gettimeofday()), is_float(gettimeofday(true)));
$l = @lcg_value();
var_dump($l >= 0 && $l < 1);
$old = get_include_path();
var_dump(set_include_path('/tmp/a:/tmp/b') === $old);
var_dump(get_include_path(), ini_get('include_path'));
var_dump(set_include_path(''));
var_dump(error_clear_last());
var_dump(is_string(gethostname()));
var_dump(gethostbyname('localhost'), gethostbyname('127.0.0.1'));
var_dump(strlen(date('Ymd')), date('Y') === date('Y', time()), date('Y', null) === date('Y'));
var_dump(mktime(0, 0, 0, 1, 1, 2000), gmdate('Y-m-d', mktime(12)) === gmdate('Y-m-d'), getdate()['year'] === (int) date('Y'));
