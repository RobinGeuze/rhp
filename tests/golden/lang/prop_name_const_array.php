<?php
// The warnings without the path: the included file's compile-time one too.
set_error_handler(function ($no, $msg, $file, $line) {
    echo "warning: $msg (", basename($file), ":$line)\n";
    return true;
});
include __DIR__ . '/prop_name_const_array.inc';
