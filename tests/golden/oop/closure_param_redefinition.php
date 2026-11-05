<?php
// Closures and arrow functions reject a repeated parameter name too.
ini_set("display_errors", "0");
register_shutdown_function(function () {
    $e = error_get_last();
    echo "last: ", $e["type"], ": ", $e["message"], " (", basename($e["file"]), ":", $e["line"], ")\n";
});
include __DIR__ . '/closure_param_redefinition.inc';
