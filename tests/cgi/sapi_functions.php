<?php
// A SAPI supplies its own functions. php-cgi's cgi-fcgi module brings
// getallheaders and the apache_* functions; php-cli's
// cli_set_process_title / cli_get_process_title do not exist here.
foreach (['cli_set_process_title', 'cli_get_process_title', 'getallheaders',
          'apache_request_headers', 'apache_response_headers', 'apache_child_terminate'] as $f) {
    echo $f, ': ', var_export(function_exists($f), true), ' ',
        var_export(in_array($f, get_extension_funcs('standard'), true), true), ' ',
        var_export(is_callable($f), true), ' ',
        var_export(in_array($f, get_defined_functions()['internal'], true), true), "\n";
}
var_dump(extension_loaded('cgi-fcgi'), extension_loaded('CGI-FCGI'), get_extension_funcs('cgi-fcgi'));
var_dump(array_slice(get_loaded_extensions(), -1));
foreach (['getallheaders', 'apache_child_terminate'] as $f) {
    $r = new ReflectionFunction($f);
    var_dump($r->getExtensionName(), $r->getNumberOfParameters());
}
try {
    cli_get_process_title();
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
var_dump(in_array('pcntl_fork', get_extension_funcs('pcntl'), true));
