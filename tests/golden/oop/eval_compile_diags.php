<?php
// eval()'d code is compiled without recording errors: the user handler sees
// its diagnostics even when the compile then fails.
ini_set("display_errors", "0");
register_shutdown_function(function () {
    $e = error_get_last();
    echo "last: ", $e["type"], ": ", $e["message"], " (", basename($e["file"]), ":", $e["line"], ")\n";
});
set_error_handler(function ($no, $msg) {
    echo "handler: $no: $msg\n";
    return true;
});
eval('class A { private function __call($a, $b) {} }');
echo "compiled\n";
eval('class B { private function __get($n) {} function __set($n) {} }');
echo "not printed\n";
