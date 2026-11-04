<?php
register_shutdown_function(function () { echo "sd\n"; });
ini_set('display_errors', '0');
throw new Exception('x');
