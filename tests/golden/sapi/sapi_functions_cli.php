<?php
// A SAPI supplies its own functions. php-cli's cli_set_process_title and
// cli_get_process_title (php files them under standard) exist here;
// php-cgi's cgi-fcgi module and its functions do not.
foreach (['cli_set_process_title', 'cli_get_process_title', 'getallheaders',
          'apache_request_headers', 'apache_response_headers', 'apache_child_terminate'] as $f) {
    echo $f, ': ', var_export(function_exists($f), true), ' ',
        var_export(in_array($f, get_extension_funcs('standard'), true), true), ' ',
        var_export(is_callable($f), true), ' ',
        var_export(in_array($f, get_defined_functions()['internal'], true), true), "\n";
}
var_dump(extension_loaded('cgi-fcgi'), get_extension_funcs('cgi-fcgi'));
var_dump(in_array('cgi-fcgi', get_loaded_extensions(), true));
var_dump((new ReflectionFunction('cli_set_process_title'))->getExtensionName());
try {
    getallheaders();
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}

// get_extension_funcs(): php's order, any case, "zend" for Core
var_dump(get_extension_funcs('json'), get_extension_funcs('JSON') === get_extension_funcs('json'));
var_dump(get_extension_funcs('ctype'), get_extension_funcs('tokenizer'));
var_dump(get_extension_funcs('zend') === get_extension_funcs('Core'));
var_dump(get_extension_funcs('Reflection'), get_extension_funcs('nope'), get_extension_funcs('zend2'));
var_dump(in_array('pcntl_fork', get_extension_funcs('pcntl'), true));
try {
    get_extension_funcs();
} catch (ArgumentCountError $e) {
    echo $e->getMessage(), "\n";
}

// Functions rhp writes in its prelude count as the module's.
foreach (['date', 'xml', 'xmlwriter'] as $m) {
    echo $m, ': ', implode(' ', get_extension_funcs($m)), "\n";
}
foreach (['hash' => ['hash_init', 'hash_update', 'hash_final', 'hash_copy'],
          'SPL' => ['iterator_apply', 'iterator_count', 'iterator_to_array'],
          'standard' => ['utf8_encode', 'utf8_decode', 'strlen']] as $m => $fs) {
    foreach ($fs as $f) {
        echo $m, ' ', $f, ': ', var_export(in_array($f, get_extension_funcs($m), true), true), "\n";
    }
}
// A module php registers no functions for.
var_dump(get_extension_funcs('xmlreader'), get_extension_funcs('Phar'));
