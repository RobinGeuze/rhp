<?php
// include converts its operand like a string cast: an object needs
// __toString, an array warns and reads as "Array".
try {
    include new stdClass();
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
set_error_handler(function ($n, $msg) {
    echo strtok($msg, ":"), "\n";
    return true;
});
var_dump(include []);
var_dump(include_once __FILE__);
echo "end\n";
