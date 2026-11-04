<?php
// php-cli's header semantics: nothing is stored, no status code, and
// headers count as sent from the first output byte. Results are collected
// and printed at the end so the first part runs before any output.
$log = [];
set_error_handler(function ($no, $msg, $file, $line) use (&$log) {
    $log[] = "E$no: " . str_replace(__DIR__ . '/', '', $msg) . " @$line";
    return true;
});
$r = [];
$r[] = [php_sapi_name(), PHP_SAPI];
$r[] = headers_sent();
$r[] = http_response_code();
header('X-Test: 1');
header('Location: /elsewhere');
$r[] = [headers_list(), http_response_code()];
$r[] = [http_response_code(201), http_response_code()];
$r[] = [setcookie('a', 'b'), setrawcookie('c', 'd', ['path' => '/'])];
header_remove('X-Test');
$r[] = headers_list();
header_remove();
$r[] = headers_list();
$r[] = header_register_callback(function () { echo "never\n"; });
$r[] = [headers_sent($file, $line), $file, $line];
$r[] = [function_exists('getallheaders'), function_exists('apache_request_headers')];
try {
    setcookie('bad=name', 'v');
} catch (ValueError $e) {
    $r[] = $e->getMessage();
}
try {
    setcookie('n', 'v', ['bogus' => 1]);
} catch (ValueError $e) {
    $r[] = $e->getMessage();
}
echo "first output\n";
$r[] = [headers_sent($file, $line), basename($file), $line];
header('X-Late: 1');
$r[] = setcookie('late', '1');
$r[] = http_response_code(500);
$r[] = headers_list();
var_dump($r);
print_r($log);
