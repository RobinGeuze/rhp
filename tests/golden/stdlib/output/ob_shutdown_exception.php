<?php
// An output handler that throws while php finalizes the buffers at request
// shutdown: the user exception handler runs right there (no frame to unwind
// to), its output lands in the failing handler's buffer, which is passed on.
set_exception_handler(function (\Throwable $e) {
    echo 'Caught: ' . $e->getMessage() . "\n";
});

register_shutdown_function(function () {
    echo "shutdown function\n";
});

ob_start(function ($buf, $phase) {
    return strtoupper($buf);
});

ob_start(function ($buf, $phase) {
    if ($phase & PHP_OUTPUT_HANDLER_FINAL) {
        throw new \Exception("final phase $phase");
    }
    return $buf;
});

echo "body\n";
