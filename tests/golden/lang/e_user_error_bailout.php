<?php
// E_USER_ERROR is fatal: the script stops (shutdown functions still run),
// unless a user handler takes it.
ini_set('display_errors', '0');
register_shutdown_function(function () {
    $e = error_get_last();
    echo "shutdown: {$e['type']} {$e['message']}\n";
});
set_error_handler(function ($l, $m) { echo "handled $l $m\n"; return true; }, E_USER_ERROR);
trigger_error('first', E_USER_ERROR);
restore_error_handler();
echo "before\n";
try {
    trigger_error('fatal', E_USER_ERROR);
} finally {
    echo "finally does not run\n";
}
echo "after\n";
