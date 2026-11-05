<?php
// Runtime ini changes whose handlers do something: max_execution_time,
// the internal encoding mbstring follows, deprecated directives; and the
// configuration hash of a run without php.ini.
set_error_handler(function ($no, $msg) {
    echo "[$no] $msg\n";
    return true;
});
var_dump(set_time_limit(5), ini_get('max_execution_time'));
var_dump(ini_set('max_execution_time', '0'), ini_get('max_execution_time'));
var_dump(mb_internal_encoding());
ini_set('internal_encoding', 'ISO-8859-1');
var_dump(mb_internal_encoding());
ini_set('internal_encoding', 'no-such-encoding');
var_dump(mb_internal_encoding());
ini_set('internal_encoding', '');
ini_set('default_charset', 'EUC-JP');
var_dump(mb_internal_encoding());
ini_set('mbstring.internal_encoding', 'SJIS');
var_dump(mb_internal_encoding());
ini_set('default_charset', 'UTF-8');
var_dump(mb_internal_encoding());
ini_set('report_memleaks', '0');
ini_set('report_memleaks', '1');
var_dump(get_cfg_var('cfg_file_path'), get_cfg_var('no.such.entry'));
var_dump(php_ini_loaded_file(), php_ini_scanned_files());
