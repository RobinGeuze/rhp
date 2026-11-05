<?php
// php_stdiop_read/write notices on a descriptor opened the other way
set_error_handler(function ($l, $m) { echo "[$l] $m\n"; return true; });
$name = __DIR__ . '/stream_errno_notices.tmp';
$h = fopen($name, 'w');
var_dump(fread($h, 10), fgets($h), fgetc($h), feof($h));
var_dump(stream_get_contents($h), fpassthru($h), fscanf($h, "%s"));
var_dump(stream_get_line($h, 10, "\n"));
fclose($h);
$h = fopen($name, 'r');
var_dump(fwrite($h, "abc"), fwrite($h, ""), ftruncate($h, 0));
fclose($h);
$h = fopen($name, 'a');
var_dump(fread($h, 3), feof($h));
fclose($h);
unlink($name);
$m = fopen('php://memory', 'w+');
fwrite($m, "012345");
var_dump(fseek($m, 2), fseek($m, -10, SEEK_CUR), ftell($m), fseek($m, -10, SEEK_END), ftell($m));
$t = fopen('php://temp', 'w+');
fwrite($t, "012345");
var_dump(fseek($t, 2), fseek($t, -10, SEEK_CUR), ftell($t), fseek($t, -10), ftell($t));
