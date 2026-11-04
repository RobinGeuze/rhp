<?php
set_error_handler(function ($no, $msg) {
    echo "[E$no] $msg\n";
    return true;
});

// A handler's locals are destructed as its frame ends, while it still counts
// as running: the destructor's output lands in the handler's own buffer
// (dropped) and triggers the "producing output" deprecation.
class D {
    function __destruct() { echo "dtor\n"; }
}
function wrap($b, $p) {
    $d = new D;
    return "M[" . $b . "|$p]\n";
}
ob_start('wrap');
echo "main";
ob_end_flush();
echo "after\n";

// An Error raised converting a handler's return value carries the builtin
// that ran the handler in its trace.
function obj($b) {
    return new stdClass;
}
ob_start('obj');
echo "x";
try {
    ob_end_flush();
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
    foreach ($e->getTrace() as $f) {
        echo "  ", $f['function'], "\n";
    }
}
echo "level ", ob_get_level(), "\n";

// The same at request shutdown.
ob_start('wrap');
echo "end";
