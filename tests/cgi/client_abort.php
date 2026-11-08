<?php
// The client reads the headers and the first line, then goes away: the
// next write fails, the loop stops, and shutdown sees the abort.
$log = getenv('CGI_LOG');
register_shutdown_function(function () use ($log) {
    file_put_contents($log, "shutdown status=" . connection_status() . " aborted=" . connection_aborted() . "\n", FILE_APPEND);
    echo "not sent\n";
});
echo str_repeat('a', 63), "\n";
flush();
for ($i = 0; $i < 50; $i++) {
    usleep(20000);
    echo "line $i\n";
}
file_put_contents($log, "loop done\n", FILE_APPEND);
