<?php
// Diagnostics raised while compiling go to the user error handler once the
// file compiled (E_COMPILE_WARNING never does).
ini_set('display_errors', '0');
set_error_handler(function ($no, $msg, $file, $line) {
    echo "handler: $no: $msg (", basename($file), ":$line)\n";
    return true;
});
register_shutdown_function(function () {
    $e = error_get_last();
    echo "last: ", $e['type'], ': ', $e['message'], ' (', basename($e['file']), ':', $e['line'], ")\n";
});
echo "start\n";
include __DIR__ . '/decl_compile_diags.inc';
