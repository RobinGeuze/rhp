<?php
// A user wrapper that replaced http:// serves every opener (no network).
class HttpMock {
    public $context;
    private $p = 0;
    function stream_open($path, $mode, $options, &$opened) { echo "open $path $mode\n"; return true; }
    function stream_read($n) { return $this->p++ ? '' : "body\n"; }
    function stream_write($d) { echo "write $d\n"; return strlen($d); }
    function stream_eof() { return $this->p > 0; }
    function stream_stat() { return []; }
    function url_stat($path, $flags) { return false; }
}
var_dump(stream_wrapper_unregister('http'));
var_dump(stream_wrapper_register('http', 'HttpMock', STREAM_IS_URL));
// listed once, last
var_dump(array_count_values(stream_get_wrappers())['http'], array_slice(stream_get_wrappers(), -1));
var_dump(file_get_contents('http://example.invalid/a'));
$f = fopen('http://example.invalid/b', 'r');
var_dump(fgets($f));
fclose($f);
var_dump(file('http://example.invalid/c'));
var_dump(readfile('http://example.invalid/d'));
var_dump(file_put_contents('http://example.invalid/e', 'data'));
var_dump(copy('http://example.invalid/f', 'http://example.invalid/g'));
