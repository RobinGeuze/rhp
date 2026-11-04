<?php
// stream_get_meta_data for pipes / memory streams / dir handles, the
// blocked flag, fseek on a pipe, stream_select on a memory stream, and
// readdir()/closedir() without a handle.
set_error_handler(function ($l, $m) { echo "[$l] $m\n"; return true; });
$p = proc_open('cat', [0 => ['pipe', 'r'], 1 => ['pipe', 'w']], $pp);
foreach ($pp as $k => $s) { echo "$k: ", json_encode(stream_get_meta_data($s)), "\n"; }
stream_set_blocking($pp[1], false);
var_dump(stream_get_meta_data($pp[1])['blocked']);
var_dump(fseek($pp[1], 0));
fclose($pp[0]); fclose($pp[1]); proc_close($p);
foreach (['r', 'w', 'a+', 'r+'] as $mode) { echo $mode, ' ', stream_get_meta_data(fopen('php://memory', $mode))['mode'], "\n"; }
$m = fopen('php://memory', 'r+'); $r = [$m]; $w = null; $e = null;
try { stream_select($r, $w, $e, 0); } catch (ValueError $x) { echo $x->getMessage(), "\n"; }
$d = opendir(__DIR__);
var_dump(array_key_exists('uri', stream_get_meta_data($d)), readdir() !== false);
closedir();
