<?php
// display_errors=0 hides the uncaught report; error_reporting masking too.
register_shutdown_function(function () { echo "shutdown ran\n"; var_dump(ini_get('display_errors')); });
ini_set('display_errors', '0');
echo "before\n";
throw new Exception('hidden');
