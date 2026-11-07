<?php
// The zlib output directives at runtime: quantity parsing and the
// headers-already-sent refusals (CLI output counts as sent).
set_error_handler(function ($no, $msg) { echo "Warning: $msg\n"; return true; });
echo "start\n";
var_dump(ini_set('zlib.output_compression', 'garbage'));
var_dump(ini_get('zlib.output_compression'));
var_dump(ini_set('zlib.output_compression', 'off'));
var_dump(ini_set('zlib.output_compression_level', '5x'));
var_dump(ini_get('zlib.output_compression_level'));
var_dump(ini_set('zlib.output_handler', 'strtoupper'));
var_dump(ini_get('zlib.output_handler'));
