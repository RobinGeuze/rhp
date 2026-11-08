<?php
register_shutdown_function(function(){ $e = error_get_last(); echo "last: ", $e['message'] ?? 'none', "\n"; var_dump(http_response_code()); });
echo "before\n";
undefined_fn();
