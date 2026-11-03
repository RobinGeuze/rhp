<?php
// A type whose class name ends in a reserved word (resolved against the
// namespace) is refused at compile time.
register_shutdown_function(function () {
    $e = error_get_last();
    echo $e['type'], " ", $e['message'], " @", $e['line'], "\n";
});
ini_set('display_errors', '0');
echo "start\n";
eval("namespace N;\nfunction f(\$a,\n  Foo\\Int \$x) {}");
