<?php
// tmpfile(): a "r+b" plain file named php… in sys_get_temp_dir(), removed
// when the stream closes.
$t = tmpfile();
$m = stream_get_meta_data($t);
var_dump($m['wrapper_type'], $m['stream_type'], $m['mode'], $m['seekable']);
$uri = $m['uri'];
var_dump(file_exists($uri), is_file($uri));
var_dump(dirname($uri) === realpath(sys_get_temp_dir()), str_starts_with(basename($uri), 'php'));
var_dump(fwrite($t, "hello"), file_get_contents($uri));
rewind($t);
var_dump(fread($t, 10), fstat($t)['size'], decoct(fstat($t)['mode'] & 0777));
fclose($t);
var_dump(file_exists($uri));

// dropping the last reference removes it too
$t = tmpfile();
$uri = stream_get_meta_data($t)['uri'];
$t = null;
var_dump(file_exists($uri));

// a leaked one goes at the end of the request
$GLOBALS['leak'] = tmpfile();
$uri = stream_get_meta_data($GLOBALS['leak'])['uri'];
register_shutdown_function(function () use ($uri) {
    var_dump(file_exists($uri));
});
