<?php
// A read of an unset superglobal is php's global fetch.
set_error_handler(function ($no, $str) { echo "warning: $str\n"; return true; });
unset($_GET);
var_dump($_GET);
var_dump(isset($_GET));
$x = 1;
unset($x);
var_dump($x);
// From a function: every scope sees it; a reference keeps the old value;
// assigned again, it goes to the end of $GLOBALS.
function drop_cookie() { unset($_COOKIE); }
function read_cookie() { return $_COOKIE; }
$_COOKIE = ['a' => 1];
$r = &$_COOKIE;
drop_cookie();
var_dump(read_cookie(), isset($_COOKIE), $r, array_key_exists('_COOKIE', $GLOBALS));
function set_cookie() { $_COOKIE = ['b' => 2]; }
set_cookie();
var_dump($_COOKIE);
$k = array_keys($GLOBALS);
var_dump(end($k));
