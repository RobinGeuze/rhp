<?php
set_exception_handler(function (Throwable $e) {
    echo 'Caught: ', $e->getMessage(), "\n";
    throw new Exception('from handler');
});
throw new Exception('main');
