<?php
// php_stat's one-entry cache over a user wrapper: a failed stat keeps the
// cached entry, an access check of a plain file (access(2)) does too, a
// stat of a plain file replaces it.
class StatCacheWrapper {
    public $context;
    function url_stat($p, $f) {
        echo "url_stat $p $f\n";
        return str_contains($p, 'yes') ? ['mode' => 0100644, 'size' => 3] : false;
    }
}
stream_wrapper_register('sc', 'StatCacheWrapper');
var_dump(file_exists('sc://yes'));
var_dump(file_exists('sc://no'));
var_dump(file_exists('sc://yes'));
var_dump(is_readable(__FILE__), file_exists(__FILE__));
var_dump(filesize('sc://yes'));
var_dump(filemtime(__FILE__) > 0);
var_dump(filesize('sc://yes'));
clearstatcache();
var_dump(is_file('sc://yes'));
// php leaves a user stream's context unset: the stream gets a fresh one
class CtxWrapper {
    public $context;
    function stream_open($path, $mode, $o, &$op) { return true; }
    function stream_eof() { return true; }
}
stream_wrapper_register('cx', 'CtxWrapper');
$c = stream_context_create(['cx' => ['a' => 1]]);
$f = fopen('cx://x', 'r', false, $c);
var_dump(stream_context_get_options($f));
var_dump(stream_get_meta_data($f)['wrapper_data']->context === $c);
