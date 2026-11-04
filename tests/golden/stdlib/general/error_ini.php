<?php
// ini_get/ini_set/error_reporting for the error-handling directives.
foreach (['display_errors', 'log_errors', 'error_log', 'html_errors', 'error_reporting',
          'error_prepend_string', 'no_such_directive'] as $k) {
    echo $k, '=', var_export(ini_get($k), true), "\n";
}
var_dump(error_reporting());
var_dump(error_reporting(E_ALL & ~E_NOTICE));
var_dump(error_reporting(null));
var_dump(ini_get('error_reporting'));
var_dump(error_reporting());
var_dump(ini_set('error_reporting', '7'), error_reporting());
var_dump(ini_set('display_errors', 'stderr'), ini_get('display_errors'));
var_dump(ini_set('display_errors', true), ini_get('display_errors'));
var_dump(ini_set('log_errors', 0), ini_get('log_errors'));
var_dump(ini_set('log_errors', null), ini_get('log_errors'));
var_dump(ini_alter('display_errors', 1.5), ini_get('display_errors'));
var_dump(ini_set('no_such_directive', 'x'), ini_get('no_such_directive'));
try {
    ini_set('display_errors', []);
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
