<?php
set_exception_handler(function ($e) { echo 'H ', $e->getMessage(), "\n"; });
register_shutdown_function(function () { echo "sd1\n"; throw new Exception('from sd'); });
register_shutdown_function(function () { echo "sd2\n"; });
throw new Exception('main');
