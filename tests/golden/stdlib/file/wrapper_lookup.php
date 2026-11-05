<?php
// php_stream_locate_url_wrapper: case, unknown schemes, file:// hosts
set_error_handler(function ($l, $m) { echo "[$l] $m\n"; return true; });
chdir(__DIR__);
file_put_contents('wrapper_lookup.tmp', 'data');
var_dump(file_get_contents('FILE://' . __DIR__ . '/wrapper_lookup.tmp'));
var_dump(file_get_contents('file://localhost' . __DIR__ . '/wrapper_lookup.tmp'));
var_dump(file_get_contents('file:////' . ltrim(__DIR__, '/') . '/wrapper_lookup.tmp'));
var_dump(file_get_contents('file://wrapper_lookup.tmp'));
var_dump(fopen('file://somehost/x', 'r'));
var_dump(file_get_contents('nosuch://x/../wrapper_lookup.tmp'));
var_dump(file_exists('nosuch://x/../wrapper_lookup.tmp'), is_file('nosuch://x/../wrapper_lookup.tmp'));
var_dump(file_get_contents('nosuch://x/../missing.tmp'));
$h = fopen('PHP://memory', 'w+');
var_dump(get_resource_type($h));
var_dump(stream_resolve_include_path('file://' . __DIR__ . '/wrapper_lookup.tmp') === __DIR__ . '/wrapper_lookup.tmp');
var_dump(stream_resolve_include_path('file://wrapper_lookup.tmp'));
mkdir('wrapper_lookup.dir');
touch('wrapper_lookup.dir/a');
touch('wrapper_lookup.dir/b');
var_dump(scandir('glob://wrapper_lookup.dir/*'));
unlink('wrapper_lookup.dir/a');
unlink('wrapper_lookup.dir/b');
var_dump(rmdir('file://' . __DIR__ . '/wrapper_lookup.dir'));
var_dump(unlink('file://' . __DIR__ . '/wrapper_lookup.tmp'));
var_dump(in_array('glob', stream_get_wrappers()));
