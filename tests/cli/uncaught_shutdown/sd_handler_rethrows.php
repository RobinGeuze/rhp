<?php
set_exception_handler(function ($e) { echo 'H ', $e->getMessage(), "\n"; throw new LogicException('again'); });
register_shutdown_function(function () { echo "sd1\n"; });
throw new Exception('main');
