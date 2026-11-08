<?php
$log = sys_get_temp_dir() . '/rhp-cgi-errlog-' . getmypid();
ini_set('error_log', $log);
@unlink($log);
error_log("hello");
trigger_error("warned", E_USER_WARNING);
echo preg_replace('/\[.*?\]/', '[T]', file_get_contents($log));
unlink($log);
// A log file that can't be opened: the SAPI's logger (stderr).
ini_set('error_log', '/nonexistent/dir/log');
error_log("fallback");
trigger_error("warned again", E_USER_WARNING);
