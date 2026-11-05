<?php
// html_errors=1 escapes a builtin's docref message as php_verror does
// (htmlspecialchars, ENT_COMPAT, in the default charset): an invalid
// sequence becomes one U+FFFD per php's decoder step, a single-byte
// charset passes bytes through. The message is escaped when it is made, so
// the error handler and error_get_last() see it escaped. trigger_error's
// messages are shown as given.
ob_start();
ini_set('html_errors', '1');
fopen("/nonexistent/\xe2\x82<b>\xf0\x9f\x98", "r");
fopen("/nonexistent/\xc3\x28 \xed\xa0\x80 \xe0\x80\xaf \xff\xfe", "r");
trigger_error("user \xe2\x82 <b>", E_USER_WARNING);
ini_set('default_charset', 'ISO-8859-1');
fopen("/nonexistent/\xe2\x82<\xe9>", "r");
ini_set('default_charset', 'Shift_JIS');
fopen("/nonexistent/\x82\xa0<\x82>", "r");
@ini_set('default_charset', 'bogus');
fopen("/nonexistent/\xe2\x82<x>", "r");
ini_set('default_charset', 'UTF-8');
ini_set('internal_encoding', 'cp1251');
fopen("/nonexistent/\xe2\x82<x>", "r");
ini_set('internal_encoding', '');
set_error_handler(function ($n, $m) { echo "handler: $m\n"; return false; });
fopen("/nonexistent/<a>\xe2\x82", "r");
var_dump(error_get_last()['message']);
restore_error_handler();
@fopen("/nonexistent/<b>&\"'", "r");
var_dump(error_get_last()['message']);
$out = ob_get_clean();
echo str_replace(__FILE__, 'FILE', $out);
