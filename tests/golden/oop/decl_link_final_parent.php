<?php
// Extending a final class is a fatal error at the class declaration,
// raised when the class is linked.
ini_set('display_errors', '0');
register_shutdown_function(function () {
    $e = error_get_last();
    echo "last: ", $e['type'], ': ', $e['message'], ' (', basename($e['file']), ':', $e['line'], ")\n";
});
echo "start\n";
final class F {}
if (true) {
    class G extends F {}
}
echo "not printed\n";
