<?php
// class_alias() validates the alias like a class name.
ini_set('display_errors', '0');
set_error_handler(function ($no, $msg, $file, $line) {
    echo "handler: $no: $msg (", basename($file), ":$line)\n";
    return true;
});
register_shutdown_function(function () {
    $e = error_get_last();
    echo "last: ", $e['type'], ': ', $e['message'], ' (', basename($e['file']), ':', $e['line'], ")\n";
});
var_dump(class_alias('stdClass', '_'));
var_dump(class_alias('stdClass', 'Foo\Mixed'));
echo "not printed\n";
