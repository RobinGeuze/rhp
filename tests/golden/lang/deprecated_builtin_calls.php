<?php
// A #[\Deprecated] builtin raises its deprecation when it is called, before
// its arguments are counted or parsed, also as a callback or through
// Reflection; an exception the handler throws stops the call.
set_error_handler(function ($no, $msg) {
    echo $no === E_DEPRECATED ? "Deprecated" : "Warning", ": $msg\n";
    return true;
});
function t(callable $c) {
    try {
        var_dump($c());
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
t(fn() => lcg_value() < 1);
t(fn() => lcg_value(1));
t(fn() => strftime([]));
t(fn() => strftime("%Y", "x"));
t(fn() => date_sunrise("x"));
t(fn() => socket_set_timeout(null, 1));
t(fn() => socket_set_timeout(STDIN, "x"));
t(fn() => zip_read(1));
t(fn() => zip_entry_name(1));
t(fn() => openssl_pkey_free(1));
t(fn() => openssl_x509_free(1));
t(fn() => curl_close(1));
t(fn() => xml_parser_free(1));
t(fn() => xml_set_object(1, 1));
t(fn() => call_user_func('lcg_value') < 1);
t(fn() => array_map('lcg_value', []));
t(fn() => (new ReflectionFunction('lcg_value'))->invoke() < 1);
$f = 'lcg_value';
t(fn() => $f() < 1);

set_error_handler(function ($no, $msg) {
    throw new ErrorException($msg, 0, $no);
});
t(fn() => strftime("%Y"));
t(fn() => socket_set_timeout(null, 1));
