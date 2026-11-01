<?php
// Stream positions on pipes (php starts them at -1: ftell false, then counts
// bytes), forward relative seeks emulated by reading, and plain-file streams
// handed to child processes.
$p = proc_open('printf abcdefghij', [1 => ['pipe', 'w'], 0 => ['pipe', 'r']], $pipes);
$r = [$pipes[1]]; $w = null; $e = null; stream_select($r, $w, $e, 5); usleep(100000);
var_dump(ftell($pipes[1]), ftell($pipes[0]), fread($pipes[1], 3), ftell($pipes[1]), fseek($pipes[1], 2, SEEK_CUR), ftell($pipes[1]), fread($pipes[1], 1));
var_dump(fseek($pipes[1], 2, SEEK_CUR), fread($pipes[1], 100), fseek($pipes[1], 5, SEEK_CUR), feof($pipes[1]));
fclose($pipes[0]); fclose($pipes[1]); proc_close($p);
$t = sys_get_temp_dir() . '/rhp_pipe_position_' . getmypid();
file_put_contents($t, "0123456789\nabc\n");
$f = fopen($t, 'r');
fgets($f);
$p = proc_open('cat', [0 => $f, 1 => ['pipe', 'w']], $pipes);
var_dump(stream_get_contents($pipes[1]));
proc_close($p);
fclose($f);
$o = fopen($t, 'w');
fwrite($o, "pre\n");
$p = proc_open('echo child', [1 => $o], $pipes);
proc_close($p);
fwrite($o, "post\n");
var_dump(ftell($o));
fclose($o);
var_dump(file_get_contents($t));
unlink($t);
