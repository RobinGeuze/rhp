<?php
register_shutdown_function(function ($a) { echo "sd1 $a\n"; register_shutdown_function(function () { echo "nested\n"; }); }, 'arg');
register_shutdown_function(function () { echo "sd2\n"; throw new Exception('in sd'); });
register_shutdown_function(function () { echo "sd3\n"; });
ob_start();
echo "buffered\n";
throw new Exception('main');
