<?php
// Every non-string argument is deprecated (php 8.1); ints still answer as
// a byte (-128..=255) or as their decimal string. High bytes classify per
// libc's C.UTF-8 table, so only digit and xdigit, false everywhere, see one.
ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
foreach ([48, 65, 33, 256, -129, 1000, -1000] as $n) {
    echo $n, ": ", (int)ctype_digit($n), (int)ctype_alpha($n), (int)ctype_graph($n),
        (int)ctype_print($n), (int)ctype_punct($n), (int)ctype_xdigit($n), "\n";
}
var_dump(ctype_digit(-60), ctype_xdigit(255));
var_dump(ctype_digit(null), ctype_alpha(true), ctype_digit(1.0), ctype_space([]),
    ctype_alnum(new stdClass));
var_dump(@ctype_cntrl(10), ctype_cntrl("\n"));
set_error_handler(function ($no, $msg) { echo "handler: $no $msg\n"; return true; });
var_dump(ctype_upper(65));
set_error_handler(function ($no, $msg) { throw new ErrorException($msg, 0, $no); });
try {
    ctype_lower(97);
} catch (ErrorException $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
