<?php
// The cases are in a strict_types include, so that the warning php raises
// while compiling it reaches the error handler.
set_error_handler(function ($no, $msg) {
    echo $no === E_DEPRECATED ? "Deprecated" : "Warning", ": $msg\n";
    return true;
});
require __DIR__ . '/in_array_ct_flag.inc';
