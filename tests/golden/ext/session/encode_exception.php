<?php
// An exception from serializing a session variable (__sleep, a Closure):
// held while the encoder finishes, later values serialize to nothing, the
// session still closes and the exception reaches the caller.
ob_start();
set_error_handler(function ($no, $msg) { echo "[$no] ", $msg, "\n"; return true; });
ini_set('session.use_cookies', 0);
ini_set('session.cache_limiter', '');
$dir = sys_get_temp_dir() . '/rhp_golden_session_encode_exception';
@mkdir($dir);
session_save_path($dir);
class B { function __sleep() { throw new Exception("sleep"); } }
class C { function __serialize(): array { echo "C::__serialize\n"; return [1]; } }

foreach (['php', 'php_binary', 'php_serialize'] as $h) {
    echo "== $h\n";
    ini_set('session.serialize_handler', $h);
    session_id('golden' . strlen($h));
    session_start();
    $_SESSION['a'] = 1;
    $_SESSION['b'] = [new B];
    $_SESSION['c'] = new C;
    $_SESSION['d'] = 'x';
    try {
        var_dump(session_write_close());
    } catch (Exception $e) {
        echo "caught ", $e->getMessage(), "\n";
    }
    var_dump(session_status());
    var_dump(bin2hex(file_get_contents("$dir/sess_golden" . strlen($h))));
    session_start();
    $_SESSION['f'] = function () {};
    try {
        var_dump(session_encode());
    } catch (Exception $e) {
        echo "caught ", $e->getMessage(), "\n";
    }
    session_abort();
}
array_map('unlink', glob("$dir/sess_*"));
rmdir($dir);
