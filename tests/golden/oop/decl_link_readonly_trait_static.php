<?php
// A static property is never readonly.
ini_set("display_errors", "0");
register_shutdown_function(function () {
    $e = error_get_last();
    echo "last: ", $e["type"], ": ", $e["message"], " (", basename($e["file"]), ":", $e["line"], ")\n";
});
trait T { public static $s; }
readonly class C { use T; }
echo "not printed\n";
