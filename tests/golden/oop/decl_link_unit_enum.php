<?php
// A class implementing UnitEnum through another interface is an E_ERROR.
ini_set('display_errors', '0');
register_shutdown_function(function () {
    $e = error_get_last();
    echo "last: ", $e['type'], ': ', $e['message'], ' (', basename($e['file']), ':', $e['line'], ")\n";
});
interface I extends UnitEnum {}
echo "start\n";
class C implements I {}
echo "not printed\n";
