<?php
// Zend OPcache is built in but not enabled under the CLI.
var_dump(extension_loaded('Zend OPcache'));
foreach (['opcache_get_status', 'opcache_invalidate', 'opcache_reset',
    'opcache_compile_file', 'opcache_is_script_cached', 'opcache_get_configuration',
    'opcache_is_script_cached_in_file_cache', 'opcache_jit_blacklist'] as $f) {
    echo $f, ': ', var_export(function_exists($f), true), "\n";
}
var_dump(opcache_get_status());
var_dump(opcache_get_status(false));
var_dump(opcache_invalidate(__FILE__));
var_dump(opcache_reset());
var_dump(opcache_is_script_cached(__FILE__));
var_dump(ini_get('opcache.enable'), ini_get('opcache.enable_cli'));
$c = opcache_get_configuration();
var_dump(is_array($c) ? array_keys($c) : $c);
var_dump(in_array('Zend OPcache', get_loaded_extensions(true)));
var_dump(in_array('Zend OPcache', get_loaded_extensions(false)));
