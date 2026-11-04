<?php
// hash_init / hash_update / hash_final / hash_copy and HashContext.
$c = hash_init('sha256');
var_dump($c instanceof HashContext, hash_update($c, 'ab'));
$d = hash_copy($c);
hash_update($d, 'c');
var_dump(hash_final($c), hash_final($d) === hash('sha256', 'abc'));
foreach ([
    fn() => hash_update($c, 'x'),
    fn() => hash_final($c),
    fn() => hash_copy($c),
    fn() => hash_init('nope'),
    fn() => hash_init('crc32', HASH_HMAC, 'k'),
    fn() => hash_init('md5', HASH_HMAC),
] as $f) {
    try { $f(); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}
$h = hash_init('MD5', HASH_HMAC, 'key');
hash_update($h, 'da');
hash_update($h, 'ta');
var_dump(hash_final($h, true) === hash_hmac('md5', 'data', 'key', true), HASH_HMAC);
$f = fopen('php://memory', 'w+');
fwrite($f, 'hello world');
rewind($f);
$h = hash_init('crc32b');
var_dump(hash_update_stream($h, $f, 5), hash_update_stream($h, $f), hash_final($h) === hash('crc32b', 'hello world'));
$h = hash_init('sha1');
var_dump(hash_update_file($h, __FILE__), hash_final($h) === sha1_file(__FILE__));
var_dump(function_exists('hash_init'), function_exists('hash_copy'));
