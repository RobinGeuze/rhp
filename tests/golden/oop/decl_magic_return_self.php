<?php
// The declared type is resolved in the class's scope before it is checked.
ini_set("display_errors", "0");
register_shutdown_function(function () {
    $e = error_get_last();
    echo "last: ", $e["type"], ": ", $e["message"], " (line ", $e["line"], ")\n";
});
eval('class A { function __toString(): self { return $this; } }');
echo "not printed\n";
