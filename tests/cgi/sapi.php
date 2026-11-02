<?php
var_dump(php_sapi_name(), PHP_SAPI);
foreach (['html_errors', 'display_errors', 'implicit_flush', 'max_execution_time', 'opcache.enable', 'cgi.fix_pathinfo', 'cgi.force_redirect', 'cgi.nph', 'fastcgi.logging', 'expose_php'] as $k) {
    echo $k, '=', var_export(ini_get($k), true), "\n";
}
var_dump(ini_get_all('cgi-fcgi'));
var_dump(extension_loaded('cgi-fcgi'), extension_loaded('Zend OPcache'));
var_dump(function_exists('getallheaders'), function_exists('apache_child_terminate'));
ini_set('html_errors', '0');
ini_restore('html_errors');
var_dump(ini_get('html_errors'));
