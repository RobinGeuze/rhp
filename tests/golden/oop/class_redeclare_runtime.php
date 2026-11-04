<?php
// Executing a nested class declaration a second time is php's fatal
// redeclaration error; the bailout marks every object destructed, so no
// destructor runs (shutdown functions still do).
ini_set('display_errors', '0');
class Noisy { function __destruct() { echo "destructor ran\n"; } }
$keep = new Noisy;
register_shutdown_function(function () {
    $e = error_get_last();
    echo $e['type'], ': ', str_replace(__DIR__ . '/', '', $e['message']), "\n";
    echo basename($e['file']), ':', $e['line'], "\n";
});
function declare_inner() {
    if (true) {
        class Inner {}
    }
    echo "declared\n";
}
declare_inner();
declare_inner();
echo "not reached\n";
