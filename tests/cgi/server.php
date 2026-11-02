<?php
foreach (['PHP_SELF', 'SCRIPT_NAME', 'SCRIPT_FILENAME', 'PATH_INFO', 'PATH_TRANSLATED', 'ORIG_PATH_INFO', 'ORIG_SCRIPT_NAME', 'ORIG_SCRIPT_FILENAME', 'ORIG_PATH_TRANSLATED', 'REQUEST_URI', 'QUERY_STRING', 'DOCUMENT_ROOT'] as $k) {
    echo $k, ' => ', var_export($_SERVER[$k] ?? null, true), "\n";
}
var_dump(is_float($_SERVER['REQUEST_TIME_FLOAT']), is_int($_SERVER['REQUEST_TIME']));
var_dump(isset($argv), isset($_SERVER['argv']), isset($_SESSION));
var_dump(getcwd() === __DIR__);
