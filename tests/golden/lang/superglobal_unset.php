<?php
// A read of an unset superglobal is php's global fetch.
set_error_handler(function ($no, $str) { echo "warning: $str\n"; return true; });
unset($_GET);
var_dump($_GET);
var_dump(isset($_GET));
$x = 1;
unset($x);
var_dump($x);
