<?php
// The generic file functions on socket streams (and pipes): positions,
// seeking, truncation, locality, line reads.
set_error_handler(function ($no, $msg) {
    if (error_reporting() & $no) { echo "[$no] $msg\n"; }
    return true;
});
[$a, $b] = stream_socket_pair(STREAM_PF_UNIX, STREAM_SOCK_STREAM, 0);
fwrite($a, "hello world\n");
var_dump(ftell($b), fread($b, 5), ftell($b), ftell($a));
var_dump(fseek($b, 1, SEEK_CUR), ftell($b));
var_dump(fseek($b, 0), rewind($b));
var_dump(fstat($b)['mode'] & 0170000);
var_dump(ftruncate($b, 0));
var_dump(fflush($a), stream_is_local($a), stream_isatty($a));
var_dump(fgetc($b), fgets($b));
var_dump(flock($a, LOCK_EX));
var_dump(stream_get_meta_data($b)['unread_bytes']);
fwrite($a, "abcdef");
var_dump(fread($b, 2), stream_get_meta_data($b)['unread_bytes']);
var_dump(stream_get_line($b, 100, "d"));
fclose($a);
var_dump(stream_get_contents($b));
var_dump(fpassthru($b), feof($b));
fclose($b);

// pipes and wrapper streams
$p = popen('echo hi', 'r');
var_dump(stream_is_local($p), fgets($p), rewind($p));
pclose($p);
$m = fopen('php://memory', 'r+');
var_dump(stream_is_local($m), rewind($m));
