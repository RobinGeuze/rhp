<?php
// php_on_timeout: shutdown functions see the timeout bit. The loop is on
// one line: where the engine notices the timeout varies run to run.
register_shutdown_function(function () {
    echo "status=", connection_status(), " aborted=", connection_aborted(), "\n";
});
set_time_limit(1);
$t = microtime(true);
while (microtime(true) - $t < 10);
