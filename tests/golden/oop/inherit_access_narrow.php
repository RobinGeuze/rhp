<?php
// Narrowing an inherited method's visibility is a compile error (raised
// before the included file runs).
ini_set('display_errors', '0');
register_shutdown_function(function () {
    $e = error_get_last();
    echo $e['type'], ': ', $e['message'], ' (', basename($e['file']), ':', $e['line'], ")\n";
});
echo "start\n";
include __DIR__ . '/inherit_access_narrow.inc';
