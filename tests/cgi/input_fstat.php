<?php
// php://input: no stat, read back after a rewind.
$f = fopen('php://input', 'r');
echo fread($f, 3), "\n";
rewind($f);
echo stream_get_contents($f), "\n";
var_dump(fstat($f));
$m = stream_get_meta_data($f);
var_dump($m['wrapper_type'], $m['stream_type'], $m['mode'], $m['uri'], $m['seekable']);
