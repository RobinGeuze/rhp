<?php
// Output buffers at request shutdown (php_request_shutdown): shutdown
// functions run first (still inside the script's buffers), then
// destructors, then every buffer is finalized top-down, each handler seeing
// PHP_OUTPUT_HANDLER_FINAL. Also: exceptions and exit() from handlers,
// output written by a handler, exit()'s message passing through buffers.
set_error_handler(function ($no, $msg) {
    echo "[E$no] $msg\n";
    return true;
});

// a handler that throws: the buffer passes through, the handler is disabled
try {
    ob_start(function ($b) { throw new Exception("boom"); });
    echo "kept ";
    ob_end_flush();
} catch (Exception $e) {
    echo "caught ", $e->getMessage(), " level=", ob_get_level(), "\n";
}
// a throwing chunk handler surfaces at the echo that filled the chunk
try {
    ob_start(function ($b, $p) {
        if ($p & PHP_OUTPUT_HANDLER_FINAL) return "<final:$b>";
        throw new Exception("chunk");
    }, 2);
    echo "a";
    echo "bc";
    echo "not reached\n";
} catch (Exception $e) {
    echo "caught ", $e->getMessage(), "\n";
}
$st = ob_get_status();
ob_end_flush();
echo "\nflags after throw: ", $st['flags'], "\n";

// output produced inside a handler goes nowhere (and is deprecated)
function side_effect($b) { echo "side"; print "effect"; return "<$b>"; }
ob_start('side_effect');
echo "main";
ob_end_flush();
echo "\n";

// the handler writes to the buffer below it
ob_start();
ob_start(fn($b) => strrev($b));
echo "olleh";
ob_end_flush();
$below = ob_get_clean();
echo "below: $below\n";

register_shutdown_function(function () {
    echo "shutdown 1 level=", ob_get_level(), "\n";
    ob_start(fn($b, $p) => "{S$p:" . $b . "}");
    echo "opened in shutdown";
});
register_shutdown_function(function () {
    echo " | shutdown 2 level=", ob_get_level(), "\n";
});
class D {
    function __destruct() { echo "dtor level=", ob_get_level(), "\n"; }
}
$d = new D;
ob_start(function ($b, $p) { return "[outer $p:" . $b . "]\n"; });
function inner($b, $p) { return "(inner $p:" . $b . ")"; }
ob_start('inner', 0, PHP_OUTPUT_HANDLER_STDFLAGS ^ PHP_OUTPUT_HANDLER_REMOVABLE);
echo "body\n";
ob_end_flush();
exit("bye\n");
