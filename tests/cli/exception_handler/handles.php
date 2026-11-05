<?php
var_dump(set_exception_handler(function ($e) { echo 'h: ', get_class($e), ' ', $e->getMessage(), "\n"; }));
var_dump(set_exception_handler('first') instanceof Closure);
restore_exception_handler();
echo "a\n";
nope();
