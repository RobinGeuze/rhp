<?php
// ignore_user_abort: the script runs to the end after the client left,
// with output off and connection_status() saying so.
$log = getenv('CGI_LOG');
ignore_user_abort(true);
register_shutdown_function(function () use ($log) {
    file_put_contents($log, "shutdown status=" . connection_status() . "\n", FILE_APPEND);
});
echo str_repeat('a', 63), "\n";
flush();
for ($i = 0; $i < 10; $i++) {
    usleep(20000);
    echo "line $i\n";
}
file_put_contents($log, "loop done status=" . connection_status() . " aborted=" . connection_aborted() . "\n", FILE_APPEND);
