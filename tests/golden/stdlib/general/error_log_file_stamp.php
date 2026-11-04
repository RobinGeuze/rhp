<?php
// error_log() into the error_log ini file prefixes php_log_err's
// "[d-M-Y H:i:s e] " timestamp, like logged errors do.
$f = tempnam(sys_get_temp_dir(), 'rhp_el');
ini_set('error_log', $f);
var_dump(error_log("hello"));
ini_set('log_errors', '1');
ini_set('display_errors', '0');
trigger_error("warned", E_USER_WARNING);
$lines = file($f);
unlink($f);
var_dump(count($lines));
foreach ($lines as $l) {
    var_dump(str_replace(__FILE__, 'FILE', preg_replace('/^\[\d{2}-[A-Z][a-z]{2}-\d{4} \d{2}:\d{2}:\d{2} [^\]]+\] /', '[STAMP] ', $l)));
}
