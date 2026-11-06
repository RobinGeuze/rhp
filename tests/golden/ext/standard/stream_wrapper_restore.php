<?php
set_error_handler(function ($no, $str) { echo "[$no] ", str_replace(__FILE__, "FILE", $str), "
"; return true; });
// stream_wrapper_unregister() / stream_wrapper_restore() on built-in wrappers
$has = fn($s) => in_array($s, stream_get_wrappers(), true);
var_dump(stream_wrapper_restore('file'));
var_dump(stream_wrapper_restore('nope'));
var_dump(stream_wrapper_unregister('nope'));
var_dump(stream_wrapper_unregister('glob'), $has('glob'));
var_dump(stream_wrapper_unregister('glob'));
var_dump(stream_wrapper_restore('glob'), $has('glob'));
var_dump(stream_wrapper_restore('glob'));
var_dump(stream_wrapper_unregister('data'), $has('data'));
var_dump(stream_wrapper_restore('data'), $has('data'));
var_dump(stream_wrapper_unregister('FILE'));
var_dump(stream_wrapper_unregister('file'), $has('file'));
var_dump(file_get_contents(__FILE__));

var_dump(stream_wrapper_restore('file'));
var_dump(strlen(file_get_contents(__FILE__)) > 0);
var_dump(stream_get_wrappers() == array_values(stream_get_wrappers()));
var_dump(stream_wrapper_unregister('file'));
var_dump(fopen('file://' . __FILE__, 'r'));
var_dump(file('nonexistent-relative.txt'));
var_dump(stream_wrapper_restore('file'));
var_dump(is_resource(fopen(__FILE__, 'r')));
