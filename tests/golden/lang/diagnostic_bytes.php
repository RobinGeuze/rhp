<?php
// Diagnostic messages are bytes end to end: a byte that is not UTF-8 in a
// message reaches the display, error_get_last(), the user error handler
// and the log as given. The output is buffered and the script path
// replaced before printing.
ob_start();
$bad = "caf\xe9\xff";
trigger_error("user $bad", E_USER_WARNING);
var_dump(bin2hex(error_get_last()["message"]));

// php_error_docref messages built from arguments
var_dump(bin2hex(pack("H*", "4\xff")));
define("C$bad", 1);
define("C$bad", 2);
var_dump(@fopen("/nonexistent-dir/$bad.txt", "r"));
var_dump(bin2hex(error_get_last()["message"]));
var_dump(fopen("/nonexistent-dir/$bad.txt", "r"));
var_dump(stat("/nonexistent-dir/$bad"));
var_dump(fopen(__FILE__, "z$bad"));
var_dump(date_default_timezone_set("Zone/$bad"));
$s = "abc";
var_dump($s["1$bad"]);

// a user error handler gets the raw bytes
set_error_handler(function ($no, $msg) {
    echo "handler: ", bin2hex($msg), "\n";
    return true;
});
trigger_error("handled $bad", E_USER_NOTICE);
var_dump(bin2hex(pack("H*", "\xfe")));
restore_error_handler();

// the log line too
$log = tempnam(sys_get_temp_dir(), "diag");
ini_set("log_errors", "1");
ini_set("display_errors", "0");
ini_set("error_log", $log);
trigger_error("logged $bad", E_USER_WARNING);
ini_set("display_errors", "1");
ini_set("log_errors", "0");
$line = file_get_contents($log);
unlink($log);
echo substr($line, strpos($line, "] ") + 2);
echo str_replace(__FILE__, "FILE", ob_get_clean());
