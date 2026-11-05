<?php
// register_shutdown_function: order, args, nested registration, ob levels,
// buffered output flushed at the end, invalid callbacks.
class K { static function st($a) { echo "K::st $a\n"; } function m() { echo "m\n"; } }
var_dump(register_shutdown_function(function ($a, $b) {
    echo "first $a $b level=", ob_get_level(), "\n";
    register_shutdown_function(function () { echo "registered during shutdown\n"; });
}, 'x', 'y'));
register_shutdown_function('printf', "printf %s\n", 'arg');
register_shutdown_function(['K', 'st'], 1);
register_shutdown_function([new K, 'm']);
register_shutdown_function('K::st', 2);
foreach (['nofn', ['NoClass', 'm'], [new stdClass, 'nomethod'], [1, 2, 3], 5, 'A::b'] as $c) {
    try {
        register_shutdown_function($c);
    } catch (TypeError $e) {
        echo $e->getMessage(), "\n";
    }
}
ob_start();
echo "still buffered at exit\n";
ob_start();
echo "nested buffer\n";
echo "end of script\n";
