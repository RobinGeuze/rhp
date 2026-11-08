<?php
set_error_handler(function ($no, $msg) {
    echo "warning: $msg\n";
    return true;
});
// proc_open() descriptors: a php://temp stream moves to a temp file
// (php_stream_temp_cast); php://memory cannot be a descriptor.
$in = fopen('php://temp', 'r+b');
fwrite($in, "hello\nworld\n");
fseek($in, 6);
$out = fopen('php://temp', 'w+b');
fwrite($out, "pre:");
$p = proc_open(['cat'], [0 => $in, 1 => $out], $pipes);
var_dump(is_resource($p), proc_close($p));
rewind($out);
var_dump(stream_get_contents($out));
$m = stream_get_meta_data($out);
var_dump($m['wrapper_type'], $m['stream_type'], $m['mode'], $m['uri']);
var_dump(fstat($out)['size']);
fwrite($out, "more");
rewind($out);
var_dump(fread($out, 100), ftell($out));
fclose($in);
fclose($out);

$mem = fopen('php://memory', 'r+b');
fwrite($mem, "mem\n");
rewind($mem);
var_dump(proc_open(['cat'], [0 => $mem, 1 => ['pipe', 'w']], $pipes));

// A second cast seeks to the stream's own position first.
$w = fopen('php://temp', 'w+b');
fwrite($w, "abc");
$p = proc_open(['echo', '1'], [1 => $w], $pipes);
proc_close($p);
var_dump(ftell($w));
$p = proc_open(['echo', '2'], [1 => $w], $pipes);
proc_close($p);
rewind($w);
var_dump(stream_get_contents($w));

// A filtered stream cannot be cast.
$f = fopen('php://temp', 'w+b');
stream_filter_append($f, 'string.toupper');
var_dump(proc_open(['echo', 'hi'], [1 => $f], $pipes));
var_dump(stream_get_meta_data($f)['stream_type']);
