<?php
// The three session serializers, the shared var_hash across session
// variables, numeric keys and bad keys, decode failures.
ob_start();
set_error_handler(function ($no, $msg) { echo "[$no] ", str_replace(__DIR__ . "/", "", $msg), "\n"; return true; });
$dir = sys_get_temp_dir() . '/rhp_golden_session_serializers';
@mkdir($dir);
session_save_path($dir);
class P { public $a = 1; public $b = [2]; }

foreach (['php', 'php_binary', 'php_serialize'] as $h) {
    echo "== $h\n";
    ini_set('session.serialize_handler', $h);
    session_id('golden' . strlen($h));
    var_dump(session_start());
    $o = new P;
    $arr = [1, 2];
    $_SESSION['obj'] = $o;
    $_SESSION['same'] = $o;
    $_SESSION['arr'] = &$arr;
    $_SESSION['ref'] = &$arr;
    $_SESSION['f'] = 1.5;
    $_SESSION[7] = 'numeric';
    $enc = session_encode();
    var_dump(str_replace("\0", '~', (string)$enc));
    session_unset();
    var_dump(session_decode($enc));
    var_dump($_SESSION);
    $_SESSION['ref'][] = 3;
    var_dump($_SESSION['arr']);
    session_destroy();
}

echo "== bad key\n";
ini_set('session.serialize_handler', 'php');
session_id('goldenbad');
session_start();
$_SESSION['a|b'] = 1;
var_dump(session_encode());
session_destroy();

echo "== decode errors\n";
session_id('goldendec');
session_start();
var_dump(session_decode('a|i:1;b|i:'));
var_dump($_SESSION, session_status());
session_start();
var_dump(session_decode('nobar'));
var_dump(session_status());
ini_set('session.serialize_handler', 'php_binary');
session_start();
var_dump(session_decode("\x05ab"));
var_dump(session_status());
ini_set('session.serialize_handler', 'php_serialize');
session_start();
var_dump(session_decode(''));
var_dump($_SESSION);
var_dump(session_decode('i:5;'));
var_dump($_SESSION);
session_destroy();
@rmdir($dir);
ob_end_flush();
