<?php
// php_on_timeout: shutdown functions see the timeout bit.
register_shutdown_function(function () {
    echo "status=", connection_status(), " aborted=", connection_aborted(), "\n";
});
set_time_limit(1);
$t = microtime(true);
while (true) {
    if (microtime(true) - $t > 10) {
        break;
    }
}
