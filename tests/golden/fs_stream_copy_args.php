<?php
// fprintf/vfprintf write to their stream; file_put_contents copies a stream
// resource from its position; stream_copy_to_stream honours $length and
// $offset; file_get_contents honours $offset and $length.
function t($f) {
    try { $r = $f(); echo "ok: ", var_export($r, true), "\n"; }
    catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}
$base = sys_get_temp_dir() . "/rhp_scpy_" . bin2hex(random_bytes(6));
mkdir($base);
$src = "$base/src.txt";
file_put_contents($src, '0123456789');

$s = fopen($src, 'r');
$d = fopen('php://memory', 'w+');
var_dump(stream_copy_to_stream($s, $d, 3, 2), ftell($s));
var_dump(stream_copy_to_stream($s, $d), ftell($s));
var_dump(stream_copy_to_stream($s, $d));
var_dump(stream_copy_to_stream($s, $d, 0));
var_dump(stream_copy_to_stream($s, $d, -1, 1));
var_dump(stream_copy_to_stream($s, $d, null, 50));
var_dump(stream_copy_to_stream($s, $d, 5, -3), ftell($s));
rewind($d);
var_dump(stream_get_contents($d));
$e = fopen('php://memory', 'w+');
var_dump(stream_copy_to_stream($e, $d));
fwrite($e, 'hello');
var_dump(stream_copy_to_stream($e, $d, 2, 10));
rewind($e);
var_dump(stream_copy_to_stream($e, $d, 2, 1), ftell($e));
t(function () use ($e) { return stream_copy_to_stream($e, 'x'); });
fclose($e);
t(function () use ($e, $d) { return stream_copy_to_stream($e, $d); });

var_dump(file_get_contents($src, false, null, 3, 4));
var_dump(file_get_contents($src, false, null, -3));
var_dump(file_get_contents($src, false, null, 20));
var_dump(file_get_contents($src, false, null, 0, 0));
var_dump(@file_get_contents($src, false, null, -11));
var_dump(file_get_contents($src, false, null, -10, 2));
var_dump(file_get_contents($src, false, null, 10, 2));
var_dump(file_get_contents($src, false, null, 2, 100));
t(function () use ($src) { return file_get_contents($src, false, null, 0, -1); });

$dst = "$base/dst.txt";
$h = fopen($src, 'r');
fseek($h, 4);
var_dump(file_put_contents($dst, $h), file_get_contents($dst), ftell($h));
var_dump(file_put_contents($dst, $h, FILE_APPEND), file_get_contents($dst));
rewind($h);
var_dump(file_put_contents($dst, $h, FILE_APPEND), file_get_contents($dst));
$m = fopen('php://memory', 'w+');
fwrite($m, 'from memory');
fseek($m, 5);
var_dump(file_put_contents($dst, $m), file_get_contents($dst));
fclose($h);
t(function () use ($dst, $h) { return file_put_contents($dst, $h); });

$f = fopen("$base/c.txt", 'w+');
fwrite($f, 'abc');
var_dump(fprintf($f, "%d-%s\n", 5, 'x'), vfprintf($f, "[%s]", ['y']));
rewind($f);
var_dump(stream_get_contents($f));
t(function () { return fprintf('nope', 'x'); });
t(function () { return vfprintf(1, 'x', []); });
fclose($f);
t(function () use ($f) { return fprintf($f, 'x'); });
$mm = fopen('php://memory', 'w+');
var_dump(fprintf($mm, '%05.1f|%x', 3.14159, 255));
rewind($mm);
var_dump(stream_get_contents($mm));
var_dump(fprintf(STDOUT, "to stdout %s\n", 'ok'));

foreach (['src.txt', 'dst.txt', 'c.txt'] as $n) unlink("$base/$n");
rmdir($base);
