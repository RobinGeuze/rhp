<?php
// php://memory / php://temp: reads past the end, failed seeks (position -1),
// ftruncate keeping php's position while clamping the buffer offset, and the
// eof flag (set only by a read that ran out of data).
$h = fopen('php://memory', 'w+');
fwrite($h, 'abc');
var_dump(fseek($h, 10), ftell($h), fread($h, 1), feof($h), fgets($h), fgetc($h), stream_get_contents($h));
var_dump(fwrite($h, 'Z'));
rewind($h);
var_dump(bin2hex(stream_get_contents($h)));
var_dump(fseek($h, -1), ftell($h), fseek($h, 2, SEEK_END), ftell($h));
var_dump(fseek($h, -20, SEEK_CUR), ftell($h), fread($h, 2), ftell($h));
var_dump(fseek($h, -50, SEEK_END), ftell($h));
var_dump(fseek($h, 1), ftell($h), fseek($h, 1, SEEK_CUR), ftell($h), fread($h, 100));

// Symfony Process's pattern on php://temp
$t = fopen('php://temp/maxmemory:1024', 'w+');
fwrite($t, 'hello world');
fseek($t, 0, SEEK_END);
fwrite($t, '!!');
var_dump(ftruncate($t, 0), ftell($t), stream_get_contents($t));
fwrite($t, 'x');
var_dump(ftell($t));
rewind($t);
var_dump(stream_get_contents($t));
$m = stream_get_meta_data($t);
var_dump(array_keys($m), $m['stream_type'], $m['uri']);

// growing and shrinking
$h = fopen('php://memory', 'w+');
fwrite($h, 'abcdef');
fseek($h, 2);
ftruncate($h, 10);
var_dump(ftell($h), bin2hex(stream_get_contents($h)));
ftruncate($h, 1);
var_dump(ftell($h), fread($h, 5));
fwrite($h, 'Q');
rewind($h);
var_dump(stream_get_contents($h));
$h = fopen('php://memory', 'w+');
fwrite($h, 'abcdef');
fseek($h, 4);
ftruncate($h, 2);
var_dump(ftell($h), feof($h));
fwrite($h, 'Q');
rewind($h);
var_dump(stream_get_contents($h));

// eof flag
$h = fopen('php://memory', 'w+');
fwrite($h, 'abc');
var_dump(feof($h));
rewind($h);
fread($h, 3);
var_dump(feof($h), stream_get_meta_data($h)['eof']);
rewind($h);
fread($h, 10);
var_dump(feof($h), stream_get_meta_data($h)['eof']);
rewind($h);
var_dump(fgets($h), feof($h));
$h = fopen('php://memory', 'w+');
fwrite($h, "a\nb");
rewind($h);
var_dump(fgets($h), feof($h), fgets($h), feof($h), fgets($h), feof($h));
rewind($h);
var_dump(fgets($h, 2), feof($h));
rewind($h);
var_dump(stream_get_contents($h), feof($h));
rewind($h);
var_dump(fgetc($h), fgetc($h), fgetc($h), feof($h), fgetc($h), feof($h));
fwrite($h, 'more');
var_dump(feof($h));
fseek($h, 0);
var_dump(feof($h));
