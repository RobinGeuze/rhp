<?php ini_set('memory_limit','8M'); register_shutdown_function(function(){ echo "sd ", error_get_last()['type'] ?? '', "\n"; }); echo "start"; $a=[]; while(true) $a[] = str_repeat('x', 1000);
