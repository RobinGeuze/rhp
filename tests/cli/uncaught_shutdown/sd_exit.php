<?php
register_shutdown_function(function () { echo "sd1\n"; exit(3); });
register_shutdown_function(function () { echo "sd2\n"; });
throw new Exception('main');
