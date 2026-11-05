<?php
// Resource ids: php's default stream context takes an id the first time a
// context-taking function (fopen, file_get_contents, ...) runs without one;
// internally opened streams use one up too.
$a = proc_open('true', [], $p);
var_dump($a);
proc_close($a);
$t = sys_get_temp_dir() . '/rhp_resource_ids_' . getmypid();
var_dump(fopen('php://memory', 'r'));
@file_get_contents($t);
var_dump(fopen('php://memory', 'r'));
file_put_contents($t, 'x');
file_get_contents($t);
var_dump(stream_context_create(), fopen($t, 'r'));
$f = fopen('php://memory', 'r');
fclose($f);
var_dump($f, fopen('php://temp', 'w'));
unlink($t);
