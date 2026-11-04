<?php
// php 8.5 deprecates reading $http_response_header at compile time, unless
// the same function assigned it (or takes it as a parameter) first. The
// cases are in the .inc, compiled (and so reported) when it is included.
set_error_handler(function ($no, $str, $file, $line) {
    echo "[$no] ", basename($file), ":$line $str\n";
    return true;
});
include __DIR__ . '/http_response_header_deprecation.inc';
echo "done\n";
