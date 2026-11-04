<?php
// iconv_get_encoding / iconv_set_encoding and the iconv.*, *_encoding and
// default_charset directives (the same on every iconv). The errno php prints
// in "Unknown error" carries over from the last failed call (EINVAL here).
set_error_handler(function ($no, $str) { echo "  W[$no]: $str\n"; return true; });
var_dump(iconv_get_encoding(), iconv_get_encoding('internal_encoding'), iconv_get_encoding('ALL'), iconv_get_encoding('bogus'));
var_dump(ini_set('default_charset', 'ISO-8859-1'), iconv_get_encoding(), iconv_strlen("\xe9t\xe9"), iconv_mime_decode("=?UTF-8?Q?=C3=A9?="));
var_dump(ini_set('internal_encoding', 'UTF-8'), iconv_get_encoding('all'), iconv_strlen("\xe9t\xe9"));
var_dump(iconv_set_encoding('internal_encoding', 'EUC-JP'), iconv_set_encoding('bogus', 'x'), iconv_set_encoding('output_encoding', str_repeat('x', 64)));
var_dump(ini_get('iconv.internal_encoding'), iconv_get_encoding());
var_dump(ini_set('iconv.input_encoding', str_repeat('y', 64)), ini_get('iconv.input_encoding'), ini_set('iconv.input_encoding', 'SJIS'));
var_dump(ini_alter('iconv.output_encoding', 'UTF-16'), iconv_set_encoding('INPUT_ENCODING', 'UTF-8'));
var_dump(ini_set('default_charset', "a\nb"), ini_set('default_charset', "a\0b"), ini_get('default_charset'));
ini_restore('iconv.internal_encoding');
var_dump(iconv_get_encoding());
var_dump(iconv_mime_encode("S", "\xa4\xa2"));
ini_set('internal_encoding', '');
ini_set('iconv.internal_encoding', '');
ini_set('default_charset', '');
var_dump(iconv_get_encoding(), iconv_strlen("é"));
var_dump(ICONV_IMPL === (PHP_OS_FAMILY === 'Darwin' ? 'libiconv' : 'glibc'), is_string(ICONV_VERSION), ICONV_MIME_DECODE_STRICT, ICONV_MIME_DECODE_CONTINUE_ON_ERROR);
var_dump(function_exists("ob_iconv_handler"), function_exists("iconv_mime_encode"));
@iconv('UTF-8', 'ASCII', "\xc3");
var_dump(iconv_mime_decode("=?UTF-8?Q?=ZZ?="));
