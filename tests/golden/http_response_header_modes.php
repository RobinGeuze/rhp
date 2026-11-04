<?php
// Which compile modes of $http_response_header php 8.5 deprecates: the
// global-namespace cases, the namespaced ones (calls resolved at run time,
// frameless functions), and eval()'d code.
set_error_handler(function ($no, $str, $file, $line) {
    echo "[$no] ", basename($file), ":$line\n";
    return true;
});
include __DIR__ . '/http_response_header_modes.inc';
include __DIR__ . '/http_response_header_modes_ns.inc';
eval('function ev1() { return $http_response_header; }');
$f = eval('return function () { return $http_response_header ?? $http_response_header; };');
echo "done\n";
